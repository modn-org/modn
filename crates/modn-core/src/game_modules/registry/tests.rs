use super::super::loader;
use super::layout::compare_versions;
use super::{ModuleRegistry, ModuleTrust};
use crate::buckets::BucketStore;
use crate::manager_roots::ManagerRoots;
use std::cmp::Ordering;
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
