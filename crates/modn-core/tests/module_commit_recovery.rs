#[path = "support/main_bucket.rs"]
mod main_bucket;

use minisign::KeyPair;
use modn_core::buckets::{
    catalog_statement, BucketStore, CatalogPublicKey, GitCatalogSource, GitCatalogSync,
    ModuleTrustAck,
};
use modn_core::data_dir;
use modn_core::game_modules::{ModuleRegistry, ReleaseOutcome};
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(arguments)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn commit(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "-m", message]);
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("copy directory");
    for entry in fs::read_dir(source).expect("copy entries") {
        let entry = entry.expect("entry");
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

fn write_signature(root: &Path, pair: &KeyPair, public_text: &str) {
    let staging = tempfile::tempdir().expect("staging");
    copy_tree(root, staging.path());
    let statement = catalog_statement(staging.path()).expect("catalog statement");
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("catalog test"),
        Some("catalog test signature"),
    )
    .expect("sign")
    .into_string();
    fs::write(root.join("catalog.pub"), public_text).expect("public key");
    fs::write(root.join("catalog.sig"), signature).expect("signature");
}

struct Catalog {
    directory: tempfile::TempDir,
    pair: KeyPair,
    public: String,
    key_id: String,
}

impl Catalog {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("remote");
        git(directory.path(), &["init", "--initial-branch=fixture"]);
        fs::write(
            directory.path().join("bucket.json"),
            json!({"schema_version": 1, "name": "community", "title": "Community"}).to_string(),
        )
        .expect("metadata");
        let pair = KeyPair::generate_unencrypted_keypair().expect("key");
        let public = pair.pk.to_box().expect("public key").into_string();
        let key_id = CatalogPublicKey::parse(&public)
            .expect("catalog key")
            .key_id()
            .to_string();
        let catalog = Self {
            directory,
            pair,
            public,
            key_id,
        };
        catalog.sign_and_commit("first");
        catalog
    }

    fn write_module(&self, game: &str, version: &str) {
        let directory = self.directory.path().join("games").join(game);
        fs::create_dir_all(&directory).expect("game module directory");
        fs::write(
            directory.join("module.json"),
            json!({
                "game_id": game,
                "version": version,
                "api_version": 1,
                "entrypoint": "module.lua",
            })
            .to_string(),
        )
        .expect("manifest");
        fs::write(
            directory.join("module.lua"),
            format!("return {{game_id='{game}', api_version=1, version='{version}'}}"),
        )
        .expect("Lua");
    }

    fn sign_and_commit(&self, message: &str) {
        write_signature(self.directory.path(), &self.pair, &self.public);
        commit(self.directory.path(), message);
    }

    fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.directory.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }

    fn sync(&self, store: &BucketStore) -> GitCatalogSync {
        store
            .sync_git_with_signer("community", Some(self.source()))
            .expect("sync")
    }

    fn trust(&self, store: &BucketStore) {
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: Some(&self.key_id),
                },
            )
            .expect("trust");
    }
}

fn write_both(catalog: &Catalog, version: &str) {
    catalog.write_module("alpha-game", version);
    catalog.write_module("beta-game", version);
}

fn active_version(data: &Path, game: &str) -> Option<String> {
    ModuleRegistry::new(data)
        .active(game)
        .expect("active release")
        .map(|release| release.module.version)
}

fn retained_versions(data: &Path, game: &str) -> Vec<(String, bool)> {
    ModuleRegistry::new(data)
        .releases(game)
        .expect("releases")
        .iter()
        .map(|release| (release.module.version.clone(), release.active))
        .collect()
}

/// Make the next `current` write for `game` fail while staging still succeeds.
fn block_active_switch(data: &Path, game: &str) {
    let directory = data.join("game-modules").join(game);
    #[cfg(windows)]
    {
        let current = directory.join("current");
        let mut permissions = fs::metadata(&current)
            .expect("current metadata")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&current, permissions).expect("read-only current");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&directory)
            .expect("module directory metadata")
            .permissions();
        permissions.set_mode(0o555);
        fs::set_permissions(&directory, permissions).expect("read-only module directory");
    }
}

fn unblock_active_switch(data: &Path, game: &str) {
    let directory = data.join("game-modules").join(game);
    #[cfg(windows)]
    {
        let current = directory.join("current");
        let mut permissions = fs::metadata(&current)
            .expect("current metadata")
            .permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&current, permissions).expect("writable current");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&directory)
            .expect("module directory metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&directory, permissions).expect("writable module directory");
    }
}

fn synchronize(store: &BucketStore) -> GitCatalogSync {
    store.sync_git_with_signer("community", None).expect("sync")
}

#[test]
fn failed_commit_restores_registry_and_next_sync_activates_v2() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.sync(&store);
        catalog.trust(&store);
        write_both(&catalog, "1.0.0");
        catalog.sign_and_commit("v1");
        let synced = synchronize(&store);
        assert_eq!(synced.modules.imported.len(), 2);
        assert_eq!(
            active_version(&data, "alpha-game").as_deref(),
            Some("1.0.0")
        );
        assert_eq!(active_version(&data, "beta-game").as_deref(), Some("1.0.0"));

        write_both(&catalog, "2.0.0");
        catalog.sign_and_commit("v2");
        block_active_switch(&data, "beta-game");
        let error = format!(
            "{:#}",
            store
                .sync_git_with_signer("community", None)
                .expect_err("commit failure")
        );
        assert!(error.contains("registry was restored"), "{error}");
        assert!(error.contains("beta-game"), "{error}");

        assert_eq!(
            active_version(&data, "alpha-game").as_deref(),
            Some("1.0.0")
        );
        assert_eq!(active_version(&data, "beta-game").as_deref(), Some("1.0.0"));
        assert_eq!(
            retained_versions(&data, "alpha-game"),
            vec![("1.0.0".to_owned(), true)]
        );
        assert_eq!(
            retained_versions(&data, "beta-game"),
            vec![("1.0.0".to_owned(), true)]
        );

        unblock_active_switch(&data, "beta-game");
        let synced = synchronize(&store);
        assert_eq!(synced.modules.imported.len(), 2);
        assert_eq!(
            synced
                .modules
                .imported
                .iter()
                .map(|report| report.outcome)
                .collect::<Vec<_>>(),
            vec![ReleaseOutcome::Imported, ReleaseOutcome::Imported]
        );
        assert_eq!(
            active_version(&data, "alpha-game").as_deref(),
            Some("2.0.0")
        );
        assert_eq!(active_version(&data, "beta-game").as_deref(), Some("2.0.0"));
        assert_eq!(
            retained_versions(&data, "alpha-game"),
            vec![("1.0.0".to_owned(), false), ("2.0.0".to_owned(), true)]
        );
        assert_eq!(
            retained_versions(&data, "beta-game"),
            vec![("1.0.0".to_owned(), false), ("2.0.0".to_owned(), true)]
        );
    });
}

#[test]
fn deliberate_rollback_survives_unchanged_catalog_sync() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.sync(&store);
        catalog.trust(&store);
        write_both(&catalog, "1.0.0");
        catalog.sign_and_commit("v1");
        synchronize(&store);
        write_both(&catalog, "2.0.0");
        catalog.sign_and_commit("v2");
        synchronize(&store);
        assert_eq!(
            active_version(&data, "alpha-game").as_deref(),
            Some("2.0.0")
        );

        ModuleRegistry::new(&data)
            .rollback("alpha-game", Some("1.0.0"))
            .expect("rollback");
        let synced = synchronize(&store);
        assert_eq!(synced.modules.imported.len(), 2);
        assert!(synced
            .modules
            .imported
            .iter()
            .all(|report| report.outcome == ReleaseOutcome::Unchanged));
        assert_eq!(
            active_version(&data, "alpha-game").as_deref(),
            Some("1.0.0")
        );
        assert_eq!(active_version(&data, "beta-game").as_deref(), Some("2.0.0"));
        assert_eq!(
            retained_versions(&data, "alpha-game"),
            vec![("1.0.0".to_owned(), true), ("2.0.0".to_owned(), false)]
        );
    });
}
