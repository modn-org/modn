#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::data_dir;
use modn_core::game_modules::{ModuleRegistry, ModuleTrust};
use std::fs;

fn source(root: &std::path::Path, game: &str, api: u32, lua_game: &str) -> std::path::PathBuf {
    let source = root.join("source");
    fs::create_dir_all(&source).expect("create fixture module");
    fs::write(
        source.join("module.json"),
        serde_json::json!({"game_id":game,"version":"1","api_version":api,"entrypoint":"main.lua"})
            .to_string(),
    )
    .expect("write fixture manifest");
    fs::write(
        source.join("main.lua"),
        format!("return {{game_id='{lua_game}', api_version=1}}"),
    )
    .expect("write fixture Lua");
    source
}

#[test]
fn rejects_official_incompatible_and_mismatched_modules_without_registration() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let registry = ModuleRegistry::new(&data_dir::data_directory().unwrap());
        for (game, api, lua_game) in [
            ("cyberpunk-2077", 1, "cyberpunk-2077"),
            ("fixture-game", 2, "fixture-game"),
            ("fixture-game", 1, "other-game"),
        ] {
            assert!(registry
                .register(
                    &source(root.path(), game, api, lua_game),
                    ModuleTrust::Acknowledged
                )
                .is_err());
            assert!(registry.list().unwrap().is_empty());
        }
    });
}

#[test]
fn refuses_duplicate_and_tampered_snapshot() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path(), "fixture-game", 1, "fixture-game");
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let registered = registry
            .register(&source, ModuleTrust::Acknowledged)
            .unwrap();
        let again = registry
            .register(&source, ModuleTrust::Acknowledged)
            .unwrap();
        assert_eq!(again, registered);
        fs::write(
            data.join("game-modules/fixture-game/versions/1/main.lua"),
            "return {}",
        )
        .unwrap();
        assert!(registry.list().is_err());
        registry.unregister("fixture-game").unwrap();
        assert!(registry.list().unwrap().is_empty());
    });
}

#[test]
fn new_module_release_identity_is_blake3() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path(), "fixture-game", 1, "fixture-game");
        let registry = ModuleRegistry::new(&data_dir::data_directory().unwrap());
        let registered = registry
            .register(&source, ModuleTrust::Acknowledged)
            .unwrap();
        let mut hasher = blake3::Hasher::new();
        hasher.update(&fs::read(source.join("module.json")).unwrap());
        hasher.update(&fs::read(source.join("main.lua")).unwrap());
        assert_eq!(
            registered.blake3,
            hasher.finalize().to_hex().to_string(),
            "the release identity must be the BLAKE3 digest of manifest and entrypoint"
        );
    });
}

#[test]
fn a_receipt_with_the_retired_sha256_digest_is_refused_without_panic() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        registry
            .register(
                &source(root.path(), "fixture-game", 1, "fixture-game"),
                ModuleTrust::Acknowledged,
            )
            .unwrap();
        let receipt = data.join("game-modules/fixture-game/versions/1/registration.json");
        let text = fs::read_to_string(&receipt).unwrap();
        let retired = text.replace("\"blake3\"", "\"sha256\"");
        assert_ne!(retired, text, "fixture record names blake3");
        fs::write(&receipt, retired).unwrap();
        let error = registry.list().expect_err("retired record must fail");
        let message = format!("{error:#}");
        assert!(message.contains("retired SHA-256"), "{message}");
        assert!(message.contains("register the module again"), "{message}");
    });
}

#[test]
fn a_pin_with_the_retired_sha256_digest_is_refused_without_panic() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        registry
            .register(
                &source(root.path(), "fixture-game", 1, "fixture-game"),
                ModuleTrust::Acknowledged,
            )
            .unwrap();
        registry.pin("fixture-game").unwrap();
        let pin = data.join("game-modules/fixture-game/pin.json");
        let text = fs::read_to_string(&pin).unwrap();
        let retired = text.replace("\"blake3\"", "\"sha256\"");
        assert_ne!(retired, text, "fixture pin names blake3");
        fs::write(&pin, retired).unwrap();
        let error = registry
            .pin_status("fixture-game")
            .expect_err("retired pin must fail");
        let message = format!("{error:#}");
        assert!(message.contains("retired SHA-256"), "{message}");
        assert!(message.contains("remove the pin"), "{message}");
    });
}

#[test]
fn copies_trusted_module_and_removes_only_registration() {
    main_bucket::seeded(|| {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(
            source.join("module.json"),
            r#"{"game_id":"fixture-game","version":"1","api_version":1,"entrypoint":"main.lua"}"#,
        )
        .unwrap();
        fs::write(
            source.join("main.lua"),
            "return {game_id='fixture-game', api_version=1}",
        )
        .unwrap();
        let registry = ModuleRegistry::new(&data_dir::data_directory().unwrap());
        assert!(registry
            .register(&source, ModuleTrust::NotAcknowledged)
            .is_err());
        assert!(registry.list().unwrap().is_empty());
        let registered = registry
            .register(&source, ModuleTrust::Acknowledged)
            .unwrap();
        assert_eq!(registered.module.game_id, "fixture-game");
        fs::remove_dir_all(&source).unwrap();
        assert_eq!(registry.list().unwrap(), vec![registered]);
        assert!(registry.unregister("../escape").is_err());
        registry.unregister("fixture-game").unwrap();
        assert!(registry.list().unwrap().is_empty());
    });
}
