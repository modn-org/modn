#[path = "support/git_command.rs"]
mod git_command;
#[path = "support/main_bucket.rs"]
mod main_bucket;

use minisign::KeyPair;
use modn_core::buckets::{
    catalog_statement, key_statement_bytes, revocation_statement, rotation_statement, BucketStore,
    CatalogPublicKey, GitCatalogSource, KeyChangeKind, ModuleTrustAck,
};
use modn_core::manager_roots::ManagerRoots;
use std::fs;
use std::io::Cursor;
use std::path::Path;

fn git(root: &Path, arguments: &[&str]) -> String {
    let output = git_command::git_command()
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

fn sign_statement(pair: &KeyPair, kind: KeyChangeKind, body: &[u8]) -> String {
    minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(key_statement_bytes(kind, "community", body)),
        Some("statement test"),
        Some("statement test signature"),
    )
    .expect("statement signature")
    .into_string()
}

fn sign_catalog(root: &Path, pair: &KeyPair) {
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
            r#"{"schema_version":2,"name":"community","title":"Community"}"#,
        )
        .expect("metadata");
        let pair = KeyPair::generate_unencrypted_keypair().expect("key");
        let public = pair.pk.to_box().expect("public key").into_string();
        let key_id = CatalogPublicKey::parse(&public)
            .expect("catalog key")
            .key_id()
            .to_string();
        Self {
            directory,
            pair,
            public,
            key_id,
        }
    }

    fn write_module(&self, version: &str) {
        let directory = self.directory.path().join("games/fixture-game");
        fs::create_dir_all(&directory).expect("module directory");
        fs::write(
            directory.join("module.json"),
            serde_json::json!({
                "game_id": "fixture-game",
                "version": version,
                "api_version": 3,
                "entrypoint": "module.lua",
            })
            .to_string(),
        )
        .expect("manifest");
        fs::write(
            directory.join("module.lua"),
            format!(
                "return {{
    game_id = 'fixture-game',
    api_version = 3,
    version = '{version}',
    marker = 'fixture',
    validate = function() return true end,
    scan = function(limit)
        return {{incomplete=false, scan_issues={{}}, redmod_available=false,
            mods={{}}, framework_signals={{}}}}
    end,
    doctor = function(input) return {{}} end,
}}"
            ),
        )
        .expect("script");
    }

    fn publish(&self, message: &str) {
        sign_catalog(self.directory.path(), &self.pair);
        fs::write(self.directory.path().join("catalog.pub"), &self.public).expect("public key");
        commit(self.directory.path(), message);
    }

    fn pin_revocation_key(&self, revocation: &KeyPair) {
        let public = revocation
            .pk
            .to_box()
            .expect("revocation key")
            .into_string();
        fs::write(self.directory.path().join("revocation.pub"), public).expect("revocation key");
    }

    fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.directory.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }

    fn trust(&self, store: &BucketStore, key_id: &str) {
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: Some(key_id),
                },
            )
            .expect("trust");
    }
}

fn next_key() -> (KeyPair, String, String) {
    let pair = KeyPair::generate_unencrypted_keypair().expect("next key");
    let public = pair.pk.to_box().expect("public key").into_string();
    let key_id = CatalogPublicKey::parse(&public)
        .expect("catalog key")
        .key_id()
        .to_string();
    (pair, public, key_id)
}

fn doctor_reports_untrusted(roots: &ManagerRoots) -> bool {
    let root = tempfile::tempdir().expect("root");
    let installation = root.path().join("game");
    fs::create_dir_all(&installation).expect("installation");
    let store =
        modn_core::state::Store::open(&root.path().join("state/state.sqlite")).expect("state");
    modn_core::inventory::doctor_with_ownership_appdata(
        roots,
        &store,
        "fixture-game",
        &installation,
        None,
    )
    .expect("doctor")
    .diagnostics
    .iter()
    .any(|diagnostic| diagnostic.code == "MODULE_SOURCE_UNTRUSTED")
}

#[test]
fn a_key_rotation_keeps_a_retained_module_release_usable() {
    main_bucket::seeded_roots(|roots| {
        let store = BucketStore::new(roots);
        let catalog = Catalog::new();
        catalog.write_module("1.0.0");
        catalog.publish("module");
        store
            .sync_git_with_signer("community", Some(catalog.source()))
            .expect("first sync");
        catalog.trust(&store, &catalog.key_id);
        content_root(roots, "fixture-game").expect("module usable before rotation");

        let (next_pair, next_public, next_key_id) = next_key();
        let body = rotation_statement(&catalog.key_id, &next_public).expect("rotation body");
        fs::write(catalog.directory.path().join("rotation.json"), &body).expect("rotation body");
        fs::write(
            catalog.directory.path().join("rotation.sig"),
            sign_statement(&catalog.pair, KeyChangeKind::Rotated, &body),
        )
        .expect("rotation signature");
        sign_catalog(catalog.directory.path(), &next_pair);
        fs::write(catalog.directory.path().join("catalog.pub"), &next_public)
            .expect("new public key");
        commit(catalog.directory.path(), "rotate the key");

        let rotated = store
            .sync_git_with_signer("community", None)
            .expect("rotation applies");
        assert_eq!(
            rotated.key_change.expect("rotation").kind,
            KeyChangeKind::Rotated
        );
        assert_eq!(rotated.revision.signer.key_id, next_key_id);
        assert_eq!(
            rotated.revision.signer.previous_signers,
            vec![catalog.key_id.clone()]
        );

        content_root(roots, "fixture-game").expect("a rotation keeps the retained release usable");
        assert!(
            !doctor_reports_untrusted(roots),
            "doctor accepts a release kept under a previous signer"
        );
        let retained = modn_core::game_modules::ModuleSource {
            bucket: Some("community".to_owned()),
            signer_key_id: Some(catalog.key_id.clone()),
        };
        let decision = store
            .module_source_trust(&retained)
            .expect("trusted source decision");
        assert!(decision.is_trusted());
        assert_eq!(
            store
                .catalog_signer("community")
                .expect("signer")
                .expect("pinned signer")
                .key_id,
            next_key_id
        );
    });
}

#[test]
fn a_revocation_keeps_the_revoked_release_untrusted() {
    main_bucket::seeded_roots(|roots| {
        let store = BucketStore::new(roots);
        let catalog = Catalog::new();
        let revocation = KeyPair::generate_unencrypted_keypair().expect("revocation key");
        catalog.pin_revocation_key(&revocation);
        catalog.write_module("1.0.0");
        catalog.publish("module");
        store
            .sync_git_with_signer("community", Some(catalog.source()))
            .expect("first sync");
        catalog.trust(&store, &catalog.key_id);
        content_root(roots, "fixture-game").expect("module usable before revocation");

        let (next_pair, next_public, next_key_id) = next_key();
        let body = revocation_statement(&catalog.key_id, &next_public).expect("revocation body");
        fs::write(catalog.directory.path().join("revocation.json"), &body)
            .expect("revocation body");
        fs::write(
            catalog.directory.path().join("revocation.sig"),
            minisign::sign(
                Some(&revocation.pk),
                &revocation.sk,
                Cursor::new(key_statement_bytes(
                    KeyChangeKind::Revoked,
                    "community",
                    &body,
                )),
                Some("statement test"),
                Some("statement test signature"),
            )
            .expect("revocation signature")
            .into_string(),
        )
        .expect("revocation signature");
        sign_catalog(catalog.directory.path(), &next_pair);
        fs::write(catalog.directory.path().join("catalog.pub"), &next_public)
            .expect("replacement key");
        commit(catalog.directory.path(), "revoke the key");

        let revoked = store
            .sync_git_with_signer("community", None)
            .expect("revocation applies");
        assert_eq!(
            revoked.key_change.expect("revocation").kind,
            KeyChangeKind::Revoked
        );
        let error = content_root(roots, "fixture-game")
            .expect_err("a revocation clears module trust")
            .to_string();
        assert!(error.contains("not a trusted module source"), "{error}");

        catalog.trust(&store, &next_key_id);
        let error = content_root(roots, "fixture-game")
            .expect_err("a release imported under the revoked key stays untrusted")
            .to_string();
        assert!(error.contains("not a trusted module source"), "{error}");
        assert!(
            doctor_reports_untrusted(roots),
            "doctor refuses a release signed by the revoked key"
        );

        catalog.write_module("2.0.0");
        sign_catalog(catalog.directory.path(), &next_pair);
        fs::write(catalog.directory.path().join("catalog.pub"), &next_public)
            .expect("replacement key");
        commit(catalog.directory.path(), "new release");
        store
            .sync_git_with_signer("community", None)
            .expect("the replacement key signs the new release");
        content_root(roots, "fixture-game")
            .expect("a release signed by the replacement key is usable");
    });
}

fn content_root(
    roots: &modn_core::manager_roots::ManagerRoots,
    game: &str,
) -> anyhow::Result<Option<modn_core::game_modules::ContentRootSpec>> {
    let selection = modn_core::game_modules::ModuleSelection::resolve(roots, game)?;
    modn_core::game_modules::content_root_spec(&selection)
}
