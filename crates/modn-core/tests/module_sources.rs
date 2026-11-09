#[path = "support/main_bucket.rs"]
mod main_bucket;

use minisign::KeyPair;
use modn_core::buckets::{
    catalog_statement, BucketStore, CatalogPublicKey, GitCatalogSource, GitCatalogSync,
    IgnoredModule, ModuleIgnoreReason, ModuleTrustAck, ModuleTrustState, TrustModulesOutcome,
};
use modn_core::data_dir;
use modn_core::game_modules::{hidden_modules, ModuleRegistry, ModuleTrust, ReleaseOutcome};
use serde_json::json;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
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

    fn write_module(&self, game: &str, version: &str, api: u32, lua: &str, root: Option<&str>) {
        let directory = self.directory.path().join("games").join(game);
        fs::create_dir_all(&directory).expect("game module directory");
        let mut manifest = json!({
            "game_id": game,
            "version": version,
            "api_version": api,
            "entrypoint": "module.lua",
        });
        if let Some(root) = root {
            manifest["content_root"] = json!({"base": "appdata", "path": root});
        }
        fs::write(directory.join("module.json"), manifest.to_string()).expect("manifest");
        fs::write(directory.join("module.lua"), lua).expect("Lua");
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

    fn trust(&self, store: &BucketStore) -> TrustModulesOutcome {
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: Some(&self.key_id),
                },
            )
            .expect("trust")
    }
}

fn policy_lua(game: &str) -> String {
    format!(
        r#"return {{
    game_id = '{game}',
    api_version = 1,
    marker = 'fixture',
    validate = function() return true end,
    scan = function(limit)
        return {{incomplete=false, scan_issues={{}}, redmod_available=false,
            mods={{}}, framework_signals={{}}}}
    end,
    doctor = function(input) return {{}} end,
}}"#
    )
}

fn fixture_module(game: &str, version: &str) -> String {
    format!("return {{game_id='{game}', api_version=1, version='{version}'}}")
}

fn active(data: &Path, game: &str) -> Option<modn_core::game_modules::RegisteredModule> {
    ModuleRegistry::new(data).active(game).expect("active release")
}

fn installation(root: &Path) -> PathBuf {
    let game = root.join("game");
    fs::create_dir_all(game.join("bin/x64")).expect("game directory");
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"fixture").expect("game marker");
    game
}

fn state(root: &Path) -> modn_core::state::Store {
    modn_core::state::Store::open(&root.join("state/state.sqlite")).expect("state")
}

#[test]
fn trusted_sync_imports_module_as_active_release() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.sync(&store);
        assert!(catalog.trust(&store).imported.is_empty());
        catalog.write_module(
            "fixture-game",
            "1.0.0",
            1,
            &fixture_module("fixture-game", "1.0.0"),
            None,
        );
        catalog.sign_and_commit("module v1");
        let synced = catalog.sync(&store);
        assert_eq!(synced.modules.imported.len(), 1);
        assert_eq!(synced.modules.imported[0].outcome, ReleaseOutcome::Imported);
        assert!(synced.modules.ignored.is_empty());
        let release = active(&data, "fixture-game").expect("active release");
        assert_eq!(release.module.version, "1.0.0");
        assert_eq!(release.source.bucket.as_deref(), Some("community"));
        assert_eq!(
            release.source.signer_key_id.as_deref(),
            Some(catalog.key_id.as_str())
        );
    });
}

#[test]
fn newer_catalog_revision_switches_and_retains_the_old_release() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.sync(&store);
        catalog.trust(&store);
        catalog.write_module(
            "fixture-game",
            "1.0.0",
            1,
            &fixture_module("fixture-game", "1.0.0"),
            None,
        );
        catalog.sign_and_commit("v1");
        catalog.sync(&store);
        catalog.write_module(
            "fixture-game",
            "2.0.0",
            1,
            &fixture_module("fixture-game", "2.0.0"),
            None,
        );
        catalog.sign_and_commit("v2");
        let synced = catalog.sync(&store);
        assert_eq!(synced.modules.imported[0].version, "2.0.0");
        let registry = ModuleRegistry::new(&data);
        let releases = registry.releases("fixture-game").expect("releases");
        assert_eq!(
            releases
                .iter()
                .map(|release| (release.module.version.as_str(), release.active))
                .collect::<Vec<_>>(),
            vec![("1.0.0", false), ("2.0.0", true)]
        );
    });
}

#[test]
fn staging_failure_leaves_the_previous_release_active() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.sync(&store);
        catalog.trust(&store);
        catalog.write_module(
            "fixture-game",
            "1.0.0",
            1,
            &fixture_module("fixture-game", "1.0.0"),
            None,
        );
        catalog.sign_and_commit("v1");
        catalog.sync(&store);
        catalog.write_module("fixture-game", "2.0.0", 1, "this is not lua", None);
        catalog.sign_and_commit("broken v2");
        let error = format!(
            "{:#}",
            store
                .sync_git_with_signer("community", None)
                .expect_err("staging failure")
        );
        assert!(error.contains("failed staging validation"), "{error}");
        let release = active(&data, "fixture-game").expect("active release");
        assert_eq!(release.module.version, "1.0.0");
        assert_eq!(
            ModuleRegistry::new(&data)
                .releases("fixture-game")
                .expect("releases")
                .len(),
            1
        );
    });
}

#[test]
fn untrusted_sync_ignores_modules_and_never_runs_lua() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.write_module("fixture-game", "1.0.0", 1, "this is not lua", None);
        catalog.sign_and_commit("broken module");
        let synced = catalog.sync(&store);
        assert!(synced.modules.imported.is_empty());
        assert_eq!(
            synced.modules.ignored,
            vec![IgnoredModule {
                game_id: "fixture-game".to_owned(),
                reason: ModuleIgnoreReason::UntrustedSource,
            }]
        );
        assert!(active(&data, "fixture-game").is_none());
        store.packages(None).expect("package listing skips modules");
        assert!(!store.list().expect("list").is_empty());
    });
}

#[test]
fn trust_imports_modules_immediately_and_returns_them() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.write_module(
            "fixture-game",
            "1.0.0",
            1,
            &fixture_module("fixture-game", "1.0.0"),
            None,
        );
        catalog.sign_and_commit("module");
        catalog.sync(&store);
        let outcome = catalog.trust(&store);
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(outcome.imported[0].outcome, ReleaseOutcome::Imported);
        assert_eq!(
            store.module_trust("community").expect("trust state"),
            ModuleTrustState::Trusted
        );
        assert_eq!(
            active(&data, "fixture-game").expect("active").module.version,
            "1.0.0"
        );
    });
}

#[test]
fn official_module_priority_hides_a_trusted_catalog_module() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.write_module(
            "cyberpunk-2077",
            "9.9.9",
            2,
            "error('hidden module must not run')",
            Some("Community/Mods"),
        );
        catalog.sign_and_commit("official collision");
        catalog.sync(&store);
        catalog.trust(&store);
        let synced = catalog.sync(&store);
        assert_eq!(
            synced.modules.ignored,
            vec![IgnoredModule {
                game_id: "cyberpunk-2077".to_owned(),
                reason: ModuleIgnoreReason::OfficialPriority,
            }]
        );
        assert!(active(&data, "cyberpunk-2077").is_none());
        let hidden = hidden_modules("cyberpunk-2077").expect("hidden modules");
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].bucket, "community");
        assert_eq!(hidden[0].version, "9.9.9");
        assert_eq!(
            modn_core::game_modules::content_root_spec("cyberpunk-2077").expect("official module"),
            None
        );

        let root = tempfile::tempdir().expect("root");
        let game = installation(root.path());
        let report =
            modn_core::inventory::doctor_with_ownership_appdata(&state(root.path()), "cyberpunk-2077", &game, None)
                .expect("doctor");
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "MODULE_HIDDEN_BY_OFFICIAL")
            .expect("hidden diagnostic");
        assert_eq!(diagnostic.severity, "info");
        assert!(diagnostic.action.is_some());
    });
}

#[test]
fn untrusted_source_is_refused_at_use_and_reported_by_doctor() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let store = BucketStore::new(&data);
        let catalog = Catalog::new();
        catalog.write_module(
            "fixture-game",
            "1.0.0",
            1,
            &policy_lua("fixture-game"),
            None,
        );
        catalog.sign_and_commit("module");
        catalog.sync(&store);
        catalog.trust(&store);
        modn_core::game_modules::content_root_spec("fixture-game").expect("trusted module");
        store.untrust_modules("community").expect("untrust");
        let error = modn_core::game_modules::content_root_spec("fixture-game")
            .expect_err("untrusted module")
            .to_string();
        assert!(
            error.contains("modn bucket trust-modules community"),
            "{error}"
        );
        assert!(error.contains("modn module remove fixture-game"), "{error}");

        let root = tempfile::tempdir().expect("root");
        let game = installation(root.path());
        let report = modn_core::inventory::doctor_with_ownership_appdata(
            &state(root.path()),
            "fixture-game",
            &game,
            None,
        )
        .expect("doctor");
        let diagnostic = report
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "MODULE_SOURCE_UNTRUSTED")
            .expect("untrusted diagnostic");
        assert_eq!(diagnostic.severity, "warning");
        let action = diagnostic.action.as_deref().expect("action");
        assert!(action.contains("trust-modules community"), "{action}");
        assert!(action.contains("module remove fixture-game"), "{action}");

        store.remove("community").expect("remove bucket");
        let error = modn_core::game_modules::content_root_spec("fixture-game")
            .expect_err("removed bucket")
            .to_string();
        assert!(
            error.contains("modn bucket trust-modules community"),
            "{error}"
        );
    });
}

#[test]
fn locally_registered_modules_still_resolve() {
    main_bucket::seeded(|| {
        let data = data_dir::data_directory().expect("data");
        let source = tempfile::tempdir().expect("source");
        let directory = source.path().join("module");
        fs::create_dir_all(&directory).expect("module directory");
        fs::write(
            directory.join("module.json"),
            json!({
                "game_id": "fixture-game",
                "version": "1.0.0",
                "api_version": 1,
                "entrypoint": "module.lua",
            })
            .to_string(),
        )
        .expect("manifest");
        fs::write(directory.join("module.lua"), policy_lua("fixture-game")).expect("Lua");
        ModuleRegistry::new(&data)
            .register(&directory, ModuleTrust::Acknowledged)
            .expect("register");
        assert_eq!(
            modn_core::game_modules::content_root_spec("fixture-game").expect("local module"),
            None
        );
    });
}
