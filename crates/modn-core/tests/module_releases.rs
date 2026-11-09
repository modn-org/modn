#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::data_dir;
use modn_core::game_modules::{ModuleRegistry, ModuleSource, ModuleTrust, ReleaseOutcome};
use std::fs;
use std::path::{Path, PathBuf};

const GAME: &str = "fixture-game";

fn source(
    root: &Path,
    name: &str,
    version: &str,
    api: u32,
    lua: &str,
    content_root: Option<&str>,
) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(&directory).expect("create fixture module");
    let mut manifest = serde_json::json!({
        "game_id": GAME,
        "version": version,
        "api_version": api,
        "entrypoint": "main.lua",
    });
    if let Some(path) = content_root {
        manifest["content_root"] = serde_json::json!({"base": "appdata", "path": path});
    }
    fs::write(directory.join("module.json"), manifest.to_string()).expect("write manifest");
    fs::write(directory.join("main.lua"), lua).expect("write Lua");
    directory
}

fn lua(api: u32) -> String {
    format!("return {{game_id='{GAME}', api_version={api}}}")
}

fn origin(bucket: &str) -> ModuleSource {
    ModuleSource {
        bucket: Some(bucket.to_owned()),
        signer_key_id: Some("0123456789ABCDEF".to_owned()),
    }
}

fn game_directory(data: &Path) -> PathBuf {
    data.join("game-modules").join(GAME)
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(directory: &Path, root: &Path, entries: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).expect("read snapshot directory") {
            let entry = entry.expect("snapshot entry");
            let path = entry.path();
            if entry.file_type().expect("snapshot type").is_dir() {
                walk(&path, root, entries);
            } else {
                entries.push((
                    path.strip_prefix(root)
                        .expect("snapshot path below root")
                        .to_path_buf(),
                    fs::read(path).expect("read snapshot file"),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

fn active_version(registry: &ModuleRegistry) -> String {
    registry
        .list()
        .expect("list active releases")
        .into_iter()
        .find(|release| release.module.game_id == GAME)
        .expect("fixture game is registered")
        .module
        .version
}

#[test]
fn register_v1_then_import_v2_keeps_v1_bytes_and_switches_active() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        let registered = registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        assert!(registered.active);
        let v1_bytes = snapshot(&game_directory(&data).join("versions").join("1.0.0"));

        let v2 = source(fixture.path(), "v2", "2.0.0", 1, &lua(1), None);
        let report = registry.import_release(&v2, origin("community")).unwrap();
        assert_eq!(report.outcome, ReleaseOutcome::Imported);
        assert_eq!(report.version, "2.0.0");

        assert_eq!(
            snapshot(&game_directory(&data).join("versions").join("1.0.0")),
            v1_bytes
        );
        let releases = registry.releases(GAME).unwrap();
        assert_eq!(
            releases
                .iter()
                .map(|release| (release.module.version.as_str(), release.active))
                .collect::<Vec<_>>(),
            vec![("1.0.0", false), ("2.0.0", true)]
        );
        assert_eq!(
            fs::read_to_string(game_directory(&data).join("current")).unwrap(),
            "2.0.0"
        );
        assert_eq!(registry.list().unwrap(), vec![releases[1].clone()]);
    });
}

#[test]
fn same_version_with_changed_digest_is_refused_and_same_digest_is_unchanged() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        let v2 = source(fixture.path(), "v2", "2.0.0", 1, &lua(1), None);
        registry.import_release(&v2, origin("community")).unwrap();
        let before = snapshot(&game_directory(&data));

        let changed = source(
            fixture.path(),
            "v2-changed",
            "2.0.0",
            1,
            "return {game_id='fixture-game', api_version=1, changed=true}",
            None,
        );
        let message = format!(
            "{:#}",
            registry
                .import_release(&changed, origin("other-bucket"))
                .unwrap_err()
        );
        assert!(message.contains("different digest"), "{message}");
        assert!(message.contains("community"), "{message}");
        assert!(message.contains("other-bucket"), "{message}");
        assert_eq!(snapshot(&game_directory(&data)), before);
        assert_eq!(active_version(&registry), "2.0.0");

        let same = source(fixture.path(), "v2-same", "2.0.0", 1, &lua(1), None);
        let report = registry.import_release(&same, origin("community")).unwrap();
        assert_eq!(report.outcome, ReleaseOutcome::Unchanged);
        assert_eq!(snapshot(&game_directory(&data)), before);
        assert_eq!(registry.releases(GAME).unwrap().len(), 2);
    });
}

#[test]
fn lower_release_is_retained_but_never_activated() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v2 = source(fixture.path(), "v2", "2.0.0", 1, &lua(1), None);
        registry.register(&v2, ModuleTrust::Acknowledged).unwrap();
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        let report = registry.import_release(&v1, origin("community")).unwrap();
        assert_eq!(report.outcome, ReleaseOutcome::NotNewer);
        assert_eq!(active_version(&registry), "2.0.0");
        let releases = registry.releases(GAME).unwrap();
        assert_eq!(releases.len(), 2);
        assert!(!releases[0].active);
        assert!(releases[1].active);
    });
}

#[test]
fn pin_blocks_import_and_rollback_until_unpinned() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        let v2 = source(fixture.path(), "v2", "2.0.0", 1, &lua(1), None);
        registry.import_release(&v2, origin("community")).unwrap();
        let pin = registry.pin(GAME).unwrap();
        assert_eq!(pin.version, "2.0.0");
        assert_eq!(pin.bucket.as_deref(), Some("community"));
        assert_eq!(registry.pin_status(GAME).unwrap(), Some(pin));

        let v3 = source(fixture.path(), "v3", "3.0.0", 1, &lua(1), None);
        let report = registry.import_release(&v3, origin("community")).unwrap();
        assert_eq!(report.outcome, ReleaseOutcome::Pinned);
        assert_eq!(report.version, "3.0.0");
        assert_eq!(active_version(&registry), "2.0.0");
        assert_eq!(registry.releases(GAME).unwrap().len(), 2);
        assert!(format!("{:#}", registry.rollback(GAME, None).unwrap_err()).contains("pinned"));

        registry.unpin(GAME).unwrap();
        assert_eq!(registry.pin_status(GAME).unwrap(), None);
        let report = registry.import_release(&v3, origin("community")).unwrap();
        assert_eq!(report.outcome, ReleaseOutcome::Imported);
        assert_eq!(active_version(&registry), "3.0.0");
    });
}

#[test]
fn rollback_switches_only_current_pointer_and_uses_next_older_release() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        for (name, version) in [("v2", "2.0.0"), ("v3", "3.0.0")] {
            let source = source(fixture.path(), name, version, 1, &lua(1), None);
            registry
                .import_release(&source, origin("community"))
                .unwrap();
        }
        let before = snapshot(&game_directory(&data));
        let current = Path::new("current");
        let unchanged = |entries: &[(PathBuf, Vec<u8>)]| {
            entries
                .iter()
                .filter(|(path, _)| path != current)
                .cloned()
                .collect::<Vec<_>>()
        };

        let rolled_back = registry.rollback(GAME, None).unwrap();
        assert_eq!(rolled_back.module.version, "2.0.0");
        assert!(rolled_back.active);
        let after = snapshot(&game_directory(&data));
        assert_eq!(unchanged(&after), unchanged(&before));
        assert_eq!(
            after
                .iter()
                .find(|(path, _)| path == current)
                .expect("current pointer exists")
                .1,
            b"2.0.0"
        );
        assert!(!data.join("state.sqlite").exists());

        let explicit = registry.rollback(GAME, Some("1.0.0")).unwrap();
        assert_eq!(explicit.module.version, "1.0.0");
        assert!(explicit.active);
        assert!(registry.rollback(GAME, Some("9.9.9")).is_err());
        assert!(format!("{:#}", registry.rollback(GAME, None).unwrap_err())
            .contains("no older retained release"));
    });
}

#[test]
fn failed_staging_validation_keeps_active_release_and_no_partial_version() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        let v2 = source(
            fixture.path(),
            "v2",
            "2.0.0",
            1,
            "error('rejected release')",
            None,
        );
        let message = format!(
            "{:#}",
            registry
                .import_release(&v2, origin("community"))
                .unwrap_err()
        );
        assert!(message.contains("rejected release"), "{message}");
        assert_eq!(active_version(&registry), "1.0.0");
        assert_eq!(registry.releases(GAME).unwrap().len(), 1);
        let versions = game_directory(&data).join("versions");
        assert!(!versions.join("2.0.0").exists());
        assert_eq!(
            fs::read_dir(&versions).unwrap().count(),
            1,
            "staging must not leave a partial version directory"
        );
        assert!(fs::read_dir(data.join("game-modules"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".staging-")));
    });
}

#[test]
fn corrupt_current_and_digest_mismatch_fail_closed() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        let v2 = source(fixture.path(), "v2", "2.0.0", 1, &lua(1), None);
        let game = game_directory(&data);
        fs::write(game.join("current"), "9.9.9").unwrap();
        assert!(registry.releases(GAME).is_err());
        assert!(registry.list().is_err());
        assert!(registry.import_release(&v2, origin("community")).is_err());

        fs::write(game.join("current"), "1.0.0").unwrap();
        fs::write(game.join("versions/1.0.0/main.lua"), "return {}").unwrap();
        let message = format!("{:#}", registry.list().unwrap_err());
        assert!(message.contains("identity or digest changed"), "{message}");
        assert!(registry.import_release(&v2, origin("community")).is_err());
    });
}

#[test]
fn legacy_unversioned_layout_instructs_re_registration() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("data");
    let legacy = data.join("game-modules").join("legacy-game");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("module.json"),
        r#"{"game_id":"legacy-game","version":"1","api_version":1,"entrypoint":"main.lua"}"#,
    )
    .unwrap();
    fs::write(legacy.join("registration.json"), "{}").unwrap();
    let registry = ModuleRegistry::new(&data);
    let message = format!("{:#}", registry.releases("legacy-game").unwrap_err());
    assert!(message.contains("legacy unversioned"), "{message}");
    assert!(message.contains("remove and register"), "{message}");
    assert!(registry.list().is_err());
}

#[test]
fn unsafe_version_strings_are_rejected() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("data");
    let registry = ModuleRegistry::new(&data);
    for (index, version) in [".", "..", ".hidden"].iter().enumerate() {
        let source = source(
            fixture.path(),
            &format!("unsafe-{index}"),
            version,
            1,
            &lua(1),
            None,
        );
        let message = format!(
            "{:#}",
            registry
                .import_release(&source, ModuleSource::default())
                .unwrap_err()
        );
        assert!(
            message.contains("invalid module version"),
            "{version}: {message}"
        );
    }
    assert!(!data.join("game-modules").join(GAME).exists());
}

#[test]
fn resolution_reads_the_active_release() {
    main_bucket::seeded(|| {
        let fixture = tempfile::tempdir().unwrap();
        let data = data_dir::data_directory().unwrap();
        let registry = ModuleRegistry::new(&data);
        let v1 = source(fixture.path(), "v1", "1.0.0", 1, &lua(1), None);
        registry.register(&v1, ModuleTrust::Acknowledged).unwrap();
        assert_eq!(
            modn_core::game_modules::content_root_spec(GAME).unwrap(),
            None
        );
        let v2 = source(
            fixture.path(),
            "v2",
            "2.0.0",
            2,
            &lua(2),
            Some("Fixture Game/Mods"),
        );
        registry
            .import_release(&v2, ModuleSource::default())
            .unwrap();
        let spec = modn_core::game_modules::content_root_spec(GAME)
            .unwrap()
            .expect("v2 declares a content root");
        assert_eq!(spec.base, "appdata");
        assert_eq!(spec.path, "Fixture Game/Mods");
        registry.rollback(GAME, None).unwrap();
        assert_eq!(
            modn_core::game_modules::content_root_spec(GAME).unwrap(),
            None
        );
    });
}
