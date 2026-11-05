#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::data_dir;
use modn_core::discovery;
use modn_core::game_modules::{with_module_source, ModuleRegistry, ModuleTrust};
use modn_core::inventory;
use modn_core::planning;
use modn_core::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const GAME: &str = "skyrim-special-edition";

fn module() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/game-modules/skyrim-special-edition")
}

fn archive(path: &Path, files: &[(&str, &[u8])]) {
    let mut writer = zip::ZipWriter::new(File::create(path).expect("create fixture ZIP"));
    for (name, bytes) in files {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .expect("create ZIP entry");
        writer.write_all(bytes).expect("write ZIP payload");
    }
    writer.finish().expect("finish fixture ZIP");
}

#[test]
fn registered_skyrim_policy_runs_managed_lifecycle_and_preserves_unrelated_files() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let game = fixture.path().join("game");
        fs::create_dir_all(game.join("Data")).unwrap();
        fs::write(game.join("SkyrimSE.exe"), b"synthetic marker").unwrap();
        fs::write(game.join("Data/manual.esp"), b"manual").unwrap();
        fs::write(game.join("plugins.txt"), b"user load order").unwrap();
        let registry = ModuleRegistry::new(&data_dir::data_directory().unwrap());
        registry
            .register(&module(), ModuleTrust::Acknowledged)
            .unwrap();
        discovery::validate_installation_for(GAME, &game).unwrap();
        let zip = fixture.path().join("mod.zip");
        archive(
            &zip,
            &[
                ("Data/Example.esp", b"plugin"),
                ("Data/textures/example/test.dds", b"texture"),
                ("Data/meshes/example/test.nif", b"mesh"),
            ],
        );
        let preview = planning::preview_install_zip(GAME, &game, &zip).unwrap();
        assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
        assert_eq!(preview.operations.len(), 3);
        assert!(!game.join("Data/Example.esp").exists());
        let mut store = Store::open(&fixture.path().join("state/state.sqlite")).unwrap();
        let installed = store.install_operations_zip(&game, &zip, &preview).unwrap();
        assert_eq!(fs::read(game.join("Data/Example.esp")).unwrap(), b"plugin");
        let scan = inventory::scan_with_ownership_for(&store, GAME, &game).unwrap();
        assert!(!scan.incomplete);
        assert_eq!(scan.mods.iter().filter(|item| item.managed).count(), 3);
        assert!(scan
            .mods
            .iter()
            .any(|item| !item.managed && item.relative_path == Path::new("Data/manual.esp")));
        let doctor = inventory::doctor_with_ownership_for(&store, GAME, &game).unwrap();
        assert!(doctor
            .diagnostics
            .iter()
            .any(|item| item.code == "SKYRIM_LOAD_ORDER_MANUAL"));
        with_module_source(Some(&fixture.path().join("missing")), || {
            assert!(discovery::validate_installation_for(GAME, &game).is_err());
        });
        with_module_source(Some(&module()), || {
            assert!(discovery::validate_installation_for("other-game", &game).is_err());
        });
        registry.unregister(GAME).unwrap();
        assert!(game.join("Data/Example.esp").exists());
        store.uninstall(&game, &installed.id).unwrap();
        assert!(!game.join("Data/Example.esp").exists());
        assert!(!game.join("Data/textures/example/test.dds").exists());
        assert_eq!(fs::read(game.join("Data/manual.esp")).unwrap(), b"manual");
        assert_eq!(
            fs::read(game.join("plugins.txt")).unwrap(),
            b"user load order"
        );
    });
}

#[test]
fn explicit_source_rejects_missing_markers_and_unsupported_payloads() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir(fixture.path().join("Data")).unwrap();
    with_module_source(Some(&module()), || {
        assert!(discovery::validate_installation_for(GAME, fixture.path()).is_err());
        fs::write(fixture.path().join("SkyrimSE.exe"), b"marker").unwrap();
        discovery::validate_installation_for(GAME, fixture.path()).unwrap();
        let zip = fixture.path().join("unsupported.zip");
        for entry in [
            "skse64_loader.exe",
            "Data/SKSE/Plugins/test.dll",
            "Data/scripts/test.pex",
            "plugins.txt",
            "Example.esp",
        ] {
            archive(&zip, &[(entry, b"unsupported")]);
            let preview = planning::preview_install_zip(GAME, fixture.path(), &zip).unwrap();
            assert!(!preview.blockers.is_empty(), "accepted {entry}");
        }
    });
}
