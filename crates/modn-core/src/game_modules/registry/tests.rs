use super::super::loader;
use super::layout::compare_versions;
use super::{activation, layout, ModuleRegistry, ModuleTrust};
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn unsafe_version_strings_are_rejected() {
    for version in [".", "..", ".hidden", "", "1/2", "1\\2"] {
        assert!(
            !loader::valid_version(version),
            "accepted unsafe version {version:?}"
        );
    }
    for version in ["1", "1.0.0", "2.0.0-beta.1", "2.0.0+build.5", "nightly"] {
        assert!(loader::valid_version(version), "rejected {version:?}");
    }
}

#[test]
fn versions_order_by_semver_when_parseable_and_bytes_otherwise() {
    assert_eq!(compare_versions("1.10.0", "1.9.0"), Ordering::Greater);
    assert_eq!(compare_versions("2.0.0", "2.0.0"), Ordering::Equal);
    assert_eq!(compare_versions("2.0.0-alpha", "2.0.0"), Ordering::Less);
    assert_eq!(compare_versions("nightly", "stable"), Ordering::Less);
    assert_eq!(compare_versions("10", "2"), Ordering::Less);
}

fn write_module_source(root: &Path, game: &str, version: &str) -> PathBuf {
    let directory = root.join(format!("source-{version}"));
    fs::create_dir_all(&directory).expect("module source");
    fs::write(
        directory.join("module.json"),
        serde_json::json!({
            "game_id": game,
            "version": version,
            "api_version": 3,
            "entrypoint": "module.lua",
        })
        .to_string(),
    )
    .expect("module manifest");
    fs::write(
        directory.join("module.lua"),
        format!("return {{game_id='{game}', api_version=3}}"),
    )
    .expect("module Lua");
    directory
}

#[test]
fn table_official_main_follows_data_and_module_state_follows_profile() {
    struct Case {
        name: &'static str,
        official_version: &'static str,
        module_version: &'static str,
    }

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../buckets/main");
    let temp = tempfile::tempdir().expect("temporary directory");
    let sources = temp.path().join("sources");
    let cases = [
        Case {
            name: "first",
            official_version: "0.3.0",
            module_version: "1.0.0",
        },
        Case {
            name: "second",
            official_version: "0.9.0",
            module_version: "2.0.0",
        },
    ];

    for case in cases {
        let data = temp.path().join(case.name).join("data");
        let profile = temp.path().join(case.name).join("profile");
        let main = data.join("buckets").join("main");
        crate::test_support::copy_tree(&fixture, &main);
        let manifest_path = main.join("games/cyberpunk-2077/module.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read official manifest"))
                .expect("official manifest");
        manifest["version"] = serde_json::json!(case.official_version);
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("write official manifest"),
        )
        .expect("write official manifest");

        let roots = ManagerRoots::new(&data, Some(&profile));
        assert_eq!(roots.official_main(), main);
        assert_eq!(roots.module_registry(), profile.join("game-modules"));
        assert_eq!(
            BucketStore::new(&roots)
                .official_module("cyberpunk-2077")
                .expect("official module path"),
            main.join("games/cyberpunk-2077")
        );
        let (official, _) = super::super::resolver::read_policy(&roots, "cyberpunk-2077")
            .expect("official module resolves");
        assert_eq!(official.version, case.official_version);

        let source = write_module_source(&sources, "fixture-game", case.module_version);
        ModuleRegistry::new(&roots)
            .register(&source, ModuleTrust::Acknowledged)
            .expect("register module");
        let (module, _) = super::super::resolver::read_policy(&roots, "fixture-game")
            .expect("registered module resolves");
        assert_eq!(module.version, case.module_version);
        assert!(profile.join("game-modules/fixture-game").is_dir());
        assert!(!data.join("game-modules").exists());
        assert!(super::super::resolver::available_games(&roots)
            .expect("available games")
            .contains(&"fixture-game".to_owned()));
    }
}

const UNDO_OLD: &str = "1.0.0";
const UNDO_NEW: &str = "2.0.0";
const UNDO_STAGING: &str = ".staging-undo-fixture";
const UNDO_GAMES: [&str; 2] = ["alpha-game", "beta-game"];
const ISOLATED_VARIABLES: [&str; 3] = ["LOCALAPPDATA", "APPDATA", "MODN_APPDATA"];

/// Point every manager app-data variable at a temporary directory.
///
/// The test builds `ManagerRoots` explicitly, but no entry point may fall back
/// to the user's real Modn state.
struct IsolatedEnvironment {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl IsolatedEnvironment {
    fn new(directory: &Path) -> Self {
        let mut previous = Vec::with_capacity(ISOLATED_VARIABLES.len());
        for name in ISOLATED_VARIABLES {
            previous.push((name, std::env::var_os(name)));
            std::env::set_var(name, directory);
        }
        Self { previous }
    }
}

impl Drop for IsolatedEnvironment {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

fn write_release(directory: &Path, game: &str, version: &str) -> String {
    fs::create_dir_all(directory).expect("release directory");
    let manifest = serde_json::json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "entrypoint": "module.lua",
    })
    .to_string();
    let script = format!("return {{game_id='{game}', api_version=3, version='{version}'}}");
    fs::write(directory.join("module.json"), manifest.as_bytes()).expect("manifest");
    fs::write(directory.join("module.lua"), script.as_bytes()).expect("script");
    let blake3 = layout::digest(manifest.as_bytes(), script.as_bytes());
    let receipt = serde_json::json!({
        "game_id": game,
        "version": version,
        "api_version": 3,
        "blake3": blake3,
        "bucket": "community",
        "signer_key_id": "fixture",
    });
    fs::write(
        directory.join(layout::RECEIPT),
        serde_json::to_vec(&receipt).expect("receipt"),
    )
    .expect("write receipt");
    blake3
}

fn staged_release(game: &str) -> PathBuf {
    Path::new(UNDO_STAGING).join(game)
}

fn current(root: &Path, game: &str) -> String {
    fs::read_to_string(root.join(game).join(layout::CURRENT))
        .expect("current pointer")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

/// Register two games on `UNDO_OLD`, stage `UNDO_NEW` for both and run the
/// commit steps up to the pointers.
fn activate_two_games(registry: &ModuleRegistry) -> activation::ActivationProgress {
    let root = registry.root();
    let mut releases = Vec::new();
    for game in UNDO_GAMES {
        let directory = root.join(game);
        write_release(
            &directory.join(layout::VERSIONS).join(UNDO_OLD),
            game,
            UNDO_OLD,
        );
        layout::write_current(&directory, UNDO_OLD).expect("old current pointer");
        let staged = staged_release(game);
        let blake3 = write_release(&root.join(&staged), game, UNDO_NEW);
        releases.push(activation::ActivationRelease {
            game_id: game.to_owned(),
            version: UNDO_NEW.to_owned(),
            blake3,
            previous: Some(UNDO_OLD.to_owned()),
            staged,
            newer: true,
        });
    }
    let mut progress = activation::ActivationProgress::default();
    activation::write_intent(&root, &releases).expect("activation intent");
    activation::move_releases(&root, &releases, &mut progress).expect("move releases");
    activation::write_current_pointers(&root, &releases, &mut progress).expect("write pointers");
    progress
}

#[test]
fn death_in_the_middle_of_an_undo_keeps_the_intent_and_the_next_open_repairs_the_set() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let _environment = IsolatedEnvironment::new(temp.path());
    let roots = ManagerRoots::new(temp.path(), None);
    let registry = ModuleRegistry::new(&roots);
    let root = registry.root();
    let progress = activate_two_games(&registry);
    assert_eq!(current(&root, UNDO_GAMES[0]), UNDO_NEW);
    assert_eq!(current(&root, UNDO_GAMES[1]), UNDO_NEW);

    // A restore that fails partway, like an I/O error, restores one pointer and
    // leaves the other on the new release.
    activation::fail_next_pointer_restore(&root);
    let undo = activation::undo_activation(&root, &progress);
    assert!(undo.is_err(), "the injected restore failure fails the undo");
    assert_eq!(
        current(&root, UNDO_GAMES[0]),
        UNDO_OLD,
        "the later undo step restored this pointer"
    );
    assert_eq!(
        current(&root, UNDO_GAMES[1]),
        UNDO_NEW,
        "the failed undo step left this pointer on the new release"
    );
    assert!(
        root.join(activation::ACTIVATION).is_file(),
        "the intent stays on disk until every undo step succeeds"
    );

    let opened = ModuleRegistry::open(&roots).expect("open repairs the partly undone set");
    assert!(matches!(
        opened.repair,
        Some(crate::game_modules::ActivationRepair::Completed { .. })
    ));
    let active = |game: &str| {
        opened
            .registry
            .active(game)
            .expect("active release")
            .expect("registered release")
            .module
            .version
    };
    let alpha = active(UNDO_GAMES[0]);
    let beta = active(UNDO_GAMES[1]);
    assert_eq!(alpha, beta, "the set is consistent, not partly switched");
    assert!(
        alpha == UNDO_OLD || alpha == UNDO_NEW,
        "the set is all old or all new, not {alpha}"
    );
    assert!(
        !root.join(activation::ACTIVATION).exists(),
        "repair removed the intent"
    );

    let reopened = ModuleRegistry::open(&roots).expect("second open");
    assert!(reopened.repair.is_none(), "repair does not repeat");
    assert_eq!(
        reopened
            .registry
            .active(UNDO_GAMES[0])
            .expect("active release")
            .expect("registered release")
            .module
            .version,
        alpha
    );
    assert_eq!(
        reopened
            .registry
            .active(UNDO_GAMES[1])
            .expect("active release")
            .expect("registered release")
            .module
            .version,
        beta
    );
}

#[test]
fn an_incomplete_rollback_keeps_the_intent_for_the_next_open() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let _environment = IsolatedEnvironment::new(temp.path());
    let roots = ManagerRoots::new(temp.path(), None);
    let registry = ModuleRegistry::new(&roots);
    let root = registry.root();
    activate_two_games(&registry);

    // Tamper with one new release so repair must roll the set back.
    let tampered = root
        .join(UNDO_GAMES[1])
        .join(layout::VERSIONS)
        .join(UNDO_NEW)
        .join("module.lua");
    fs::write(&tampered, b"return {game_id='beta-game', api_version=3}").expect("tamper script");
    // Block one `current` restore while the rollback runs.
    let blocked = root.join(UNDO_GAMES[0]).join(layout::CURRENT);
    fs::remove_file(&blocked).expect("remove current pointer");
    fs::create_dir(&blocked).expect("block the current pointer");

    let opened = ModuleRegistry::open(&roots).expect("first open");
    match opened.repair {
        Some(crate::game_modules::ActivationRepair::RolledBack { reason, .. }) => {
            assert!(reason.contains("rollback incomplete"), "{reason}");
        }
        other => panic!("expected an incomplete rollback, got {other:?}"),
    }
    assert_eq!(
        current(&root, UNDO_GAMES[1]),
        UNDO_OLD,
        "the rollback reached the release that could be restored"
    );
    assert!(
        root.join(activation::ACTIVATION).is_file(),
        "an incomplete rollback keeps the intent"
    );

    // Clear the block; the next open finishes the rollback.
    fs::remove_dir(&blocked).expect("unblock the current pointer");
    let reopened = ModuleRegistry::open(&roots).expect("second open");
    match reopened.repair {
        Some(crate::game_modules::ActivationRepair::RolledBack { reason, .. }) => {
            assert!(!reason.contains("rollback incomplete"), "{reason}");
        }
        other => panic!("expected a rollback, got {other:?}"),
    }
    for game in UNDO_GAMES {
        assert_eq!(current(&root, game), UNDO_OLD);
        assert_eq!(
            reopened
                .registry
                .active(game)
                .expect("active release")
                .expect("registered release")
                .module
                .version,
            UNDO_OLD
        );
    }
    assert!(
        !root.join(activation::ACTIVATION).exists(),
        "the completed rollback removed the intent"
    );
}
