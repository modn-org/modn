#[path = "support/catalog_git.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;

use modn_core::buckets::{BucketStore, ModuleTrustAck, ModuleTrustState};
use std::fs;
use support::{package, registration, signed_remote, write_module_files};

#[test]
fn trust_refuses_an_invalid_trusted_games_layout_and_records_no_trust() {
    main_bucket::seeded(|| {
        let invalid: [(&str, bool); 3] = [
            ("games/test-game/notes.md", false),
            ("games/loose.md", false),
            ("games/test-game/nested", true),
        ];
        for (relative, directory) in invalid {
            let remote = signed_remote(|root| {
                let offending = root.join(relative);
                if directory {
                    fs::create_dir_all(&offending).expect("directory entry");
                    fs::write(offending.join("file.txt"), "extra").expect("nested file");
                } else {
                    fs::create_dir_all(offending.parent().expect("parent")).expect("parent");
                    fs::write(&offending, "extra").expect("file entry");
                }
            });
            let state = tempfile::tempdir().expect("state");
            let store = BucketStore::new(&main_bucket::roots(state.path()));
            store
                .sync_git_with_signer("community", Some(remote.source()))
                .expect("first sync ignores untrusted games");
            let registration_path = registration(state.path());
            let before = fs::read(&registration_path).expect("registration");
            let published = relative.replace('/', std::path::MAIN_SEPARATOR_STR);
            let error = store
                .trust_modules(
                    "community",
                    ModuleTrustAck::Acknowledged {
                        expected_fingerprint: None,
                    },
                )
                .expect_err("invalid trusted layout");
            let message = format!("{error:#}");
            assert!(message.contains("bucket community"), "{message}");
            assert!(message.contains(&published), "{message}");
            assert_eq!(
                fs::read(&registration_path).expect("registration"),
                before,
                "refusal writes no registration"
            );
            assert_eq!(
                store.module_trust("community").expect("trust state"),
                ModuleTrustState::Untrusted
            );
            assert!(
                !store
                    .list()
                    .expect("list")
                    .iter()
                    .find(|bucket| bucket.name == "community")
                    .expect("community")
                    .module_trusted
            );
        }
    });
}

#[test]
fn trust_accepts_a_valid_games_layout() {
    main_bucket::seeded(|| {
        let remote = signed_remote(|root| write_module_files(root, "test-game", 3));
        let state = tempfile::tempdir().expect("state");
        let store = BucketStore::new(&main_bucket::roots(state.path()));
        store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("first sync");
        let outcome = store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: None,
                },
            )
            .expect("valid layout is trusted");
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(
            store.module_trust("community").expect("trust state"),
            ModuleTrustState::Trusted
        );
        assert!(
            store
                .list()
                .expect("list")
                .iter()
                .find(|bucket| bucket.name == "community")
                .expect("community")
                .module_trusted
        );
    });
}

#[test]
fn trusted_catalogs_refuse_a_package_manifest_under_games() {
    main_bucket::isolated_profile(|profile| {
        for (relative, contents) in [
            ("games/test-game/extra.json", package()),
            ("games/test-game/notes.txt", "notes".to_owned()),
            ("games/loose.json", package()),
        ] {
            let remote = signed_remote(|_| {});
            let store = BucketStore::new(&main_bucket::roots(profile));
            store
                .sync_git_with_signer("community", Some(remote.source()))
                .expect("first sync");
            store
                .trust_modules(
                    "community",
                    ModuleTrustAck::Acknowledged {
                        expected_fingerprint: None,
                    },
                )
                .expect("trust");
            fs::create_dir_all(remote.path().join("games/test-game")).expect("games");
            fs::write(remote.path().join(relative), contents).expect("extra entry");
            remote.resign(relative);
            let error = store.sync_git("community", None).expect_err(relative);
            let error = format!("{error:#}");
            assert!(error.contains("games/<game-id>"), "{relative}: {error}");
            assert_eq!(
                store.module_trust("community").expect("trust"),
                ModuleTrustState::Trusted
            );
            store.remove("community").expect("reset fixture");
        }

        let remote = signed_remote(|_| {});
        let store = BucketStore::new(&main_bucket::roots(profile));
        store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("first sync");
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: None,
                },
            )
            .expect("trust");
        fs::create_dir_all(remote.path().join("games/test-game/nested")).expect("nested");
        fs::write(remote.path().join("games/test-game/nested/file.txt"), "x").expect("nested file");
        remote.resign("nested");
        let error = store
            .sync_git("community", None)
            .expect_err("nested directory");
        assert!(
            format!("{error:#}").contains("games/<game-id>"),
            "{error:#}"
        );
        assert!(
            profile.join("game-modules/registry.lock").is_file(),
            "trust_modules locks the isolated profile"
        );
    });
}

#[test]
fn untrusted_catalogs_ignore_games_and_stay_untrusted() {
    main_bucket::seeded(|| {
        let remote = signed_remote(|root| {
            fs::create_dir_all(root.join("games/test-game")).expect("games");
            fs::write(
            root.join("games/test-game/module.json"),
            r#"{"game_id":"test-game","version":"1","api_version":3,"entrypoint":"module.lua"}"#,
        )
        .expect("module manifest");
            fs::write(root.join("games/test-game/module.lua"), "return {}").expect("module script");
            fs::write(root.join("games/loose.txt"), "ignored").expect("loose entry");
        });
        let state = tempfile::tempdir().expect("state");
        let store = BucketStore::new(&main_bucket::roots(state.path()));
        let synced = store
            .sync_git_with_signer("community", Some(remote.source()))
            .expect("untrusted games folder is not executed");
        assert_eq!(synced.modules.ignored.len(), 1);
        assert_eq!(synced.modules.ignored[0].game_id, "test-game");
        assert_eq!(
            synced.modules.ignored[0].reason,
            modn_core::buckets::ModuleIgnoreReason::UntrustedSource
        );
        assert_eq!(
            store.module_trust("community").expect("trust"),
            ModuleTrustState::Untrusted
        );
        assert!(store.trusted_module_sources().expect("sources").is_empty());
        let packages = store.packages(Some("test-game")).expect("packages");
        assert!(packages.iter().any(|package| package.package.id == "mod"));
    });
}
