#[path = "support/main_bucket.rs"]
mod main_bucket;
use minisign::KeyPair;
use modn_core::buckets::{
    catalog_statement, BucketStore, CatalogPublicKey, GitCatalogRevision, GitCatalogSource,
    ModuleTrustAck, ModuleTrustState,
};
use serde_json::{json, Value};
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

fn package() -> String {
    json!({"schema_version":2,"id":"mod","game":"test-game","name":"Mod","version":"1",
        "archive_format":"zip","blake3":"0".repeat(64),
        "sources":[{"kind":"manual","filename":"mod.zip"}]})
    .to_string()
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

struct TestRemote {
    directory: tempfile::TempDir,
    key_id: String,
}

impl TestRemote {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("remote");
        git(directory.path(), &["init", "--initial-branch=fixture"]);
        fs::write(
            directory.path().join("bucket.json"),
            r#"{"schema_version":2,"name":"community","title":"Community"}"#,
        )
        .expect("metadata");
        fs::create_dir_all(directory.path().join("mods/test-game")).expect("games");
        fs::write(directory.path().join("mods/test-game/mod.json"), package()).expect("package");
        let pair = KeyPair::generate_unencrypted_keypair().expect("key");
        let public_text = pair.pk.to_box().expect("public key").into_string();
        let key_id = CatalogPublicKey::parse(&public_text)
            .expect("catalog key")
            .key_id()
            .to_string();
        write_signature(directory.path(), &pair, &public_text);
        commit(directory.path(), "first");
        Self { directory, key_id }
    }

    fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.directory.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }

    fn sync(&self, store: &BucketStore) -> GitCatalogRevision {
        store
            .sync_git_with_signer("community", Some(self.source()))
            .expect("sync")
            .revision
    }
}

fn fixture() -> (
    tempfile::TempDir,
    BucketStore,
    TestRemote,
    GitCatalogRevision,
) {
    let remote = TestRemote::new();
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let revision = remote.sync(&store);
    (state, store, remote, revision)
}

fn registration(state: &Path) -> PathBuf {
    state.join("buckets/registrations/community.json")
}

fn local_bucket(root: &Path) -> PathBuf {
    let bucket = root.join("local-bucket");
    fs::create_dir_all(bucket.join("games/test-game")).expect("games");
    fs::write(
        bucket.join("bucket.json"),
        json!({"schema_version": 2, "name": "local", "title": "Local"}).to_string(),
    )
    .expect("metadata");
    bucket
}

fn other_key_id() -> String {
    let pair = KeyPair::generate_unencrypted_keypair().expect("key");
    let text = pair.pk.to_box().expect("public key").into_string();
    CatalogPublicKey::parse(&text)
        .expect("catalog key")
        .key_id()
        .to_string()
}

#[test]
fn acknowledged_trust_records_the_pinned_signer() {
    let (state, store, _remote, revision) = fixture();
    let outcome = store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: Some(&revision.signer.key_id),
            },
        )
        .expect("trust");
    assert_eq!(outcome.bucket, "community");
    assert_eq!(outcome.signer_key_id, revision.signer.key_id);
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Trusted
    );
    let sources = store.trusted_module_sources().expect("sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].bucket, "community");
    assert_eq!(sources[0].signer_key_id, revision.signer.key_id);
    let registration: Value =
        serde_json::from_slice(&fs::read(registration(state.path())).expect("registration"))
            .expect("JSON");
    assert_eq!(
        registration["module_trust"]["signer_key_id"].as_str(),
        Some(revision.signer.key_id.as_str())
    );
}

#[test]
fn unacknowledged_or_mismatched_trust_writes_nothing() {
    let (state, store, _remote, revision) = fixture();
    let path = registration(state.path());
    let before = fs::read(&path).expect("registration");

    let error = store
        .trust_modules("community", ModuleTrustAck::NotAcknowledged)
        .expect_err("refuse unacknowledged trust")
        .to_string();
    assert!(error.contains("acknowledgement"), "{error}");
    assert_eq!(fs::read(&path).expect("registration"), before);
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Untrusted
    );

    let mismatch = other_key_id();
    let error = store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: Some(&mismatch),
            },
        )
        .expect_err("refuse fingerprint mismatch")
        .to_string();
    assert!(error.contains("fingerprint mismatch"), "{error}");
    assert!(error.contains(&revision.signer.key_id), "{error}");
    assert_eq!(fs::read(&path).expect("registration"), before);
    assert!(store.trusted_module_sources().expect("sources").is_empty());
}

#[test]
fn local_folder_buckets_cannot_be_module_sources() {
    let state = tempfile::tempdir().expect("state");
    let source = tempfile::tempdir().expect("source");
    let bucket = local_bucket(source.path());
    let store = BucketStore::new(state.path());
    store.add("local", &bucket).expect("add");
    let path = state.path().join("buckets/registrations/local.json");
    let before = fs::read(&path).expect("registration");

    let error = store
        .trust_modules(
            "local",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect_err("refuse local trust")
        .to_string();
    assert_eq!(
        error,
        "local folder buckets cannot be module sources: local"
    );
    assert_eq!(fs::read(&path).expect("registration"), before);
    assert_eq!(
        store.module_trust("local").expect("trust state"),
        ModuleTrustState::Local
    );
    assert!(store.untrust_modules("local").is_err());
}

#[test]
fn unknown_bucket_trust_writes_nothing() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    let error = store
        .trust_modules(
            "missing",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect_err("refuse unknown bucket")
        .to_string();
    assert!(
        error.contains("bucket is not registered: missing"),
        "{error}"
    );
    assert!(!state.path().join("buckets").exists());
    assert!(store.untrust_modules("missing").is_err());
}

#[test]
fn main_is_always_trusted_and_rejects_trust_records() {
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
    assert_eq!(
        store.module_trust("main").expect("main state"),
        ModuleTrustState::Main
    );
    let error = store
        .trust_modules(
            "main",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect_err("refuse main trust record")
        .to_string();
    assert!(error.contains("official main bucket"), "{error}");
    let error = store
        .untrust_modules("main")
        .expect_err("refuse main untrust")
        .to_string();
    assert!(error.contains("official main bucket"), "{error}");
    assert!(!state.path().join("buckets").exists());
}

#[test]
fn git_bucket_without_a_pinned_signer_cannot_be_trusted() {
    let (state, store, _remote, _revision) = fixture();
    let path = registration(state.path());
    let mut registration: Value =
        serde_json::from_slice(&fs::read(&path).expect("registration")).expect("JSON");
    registration["git"]
        .as_object_mut()
        .expect("git")
        .remove("signer");
    fs::write(&path, serde_json::to_vec(&registration).expect("JSON")).expect("write");
    let before = fs::read(&path).expect("registration");

    let error = store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect_err("refuse missing pin")
        .to_string();
    assert!(
        error.contains("registered before catalog signing"),
        "{error}"
    );
    assert_eq!(fs::read(&path).expect("registration"), before);
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Untrusted
    );
}

#[test]
fn untrust_removes_only_the_trust_record() {
    let (state, store, _remote, _revision) = fixture();
    store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect("trust");
    store.untrust_modules("community").expect("untrust");
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Untrusted
    );
    assert!(store.trusted_module_sources().expect("sources").is_empty());
    let registration: Value =
        serde_json::from_slice(&fs::read(registration(state.path())).expect("registration"))
            .expect("JSON");
    assert!(registration.get("module_trust").is_none());
    assert!(registration.get("git").is_some());
    assert!(registration.get("path").is_some());
    assert!(store.git_revision("community").expect("revision").is_some());
    store
        .untrust_modules("community")
        .expect("idempotent untrust");
}

#[test]
fn changed_pinned_signer_ends_trust() {
    let (state, store, _remote, _revision) = fixture();
    store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect("trust");
    let path = registration(state.path());
    let mut registration: Value =
        serde_json::from_slice(&fs::read(&path).expect("registration")).expect("JSON");
    registration["git"]["signer"]["key_id"] = json!(other_key_id());
    fs::write(&path, serde_json::to_vec(&registration).expect("JSON")).expect("write");
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Untrusted
    );
    assert!(store.trusted_module_sources().expect("sources").is_empty());
}

#[test]
fn untrusted_registrations_omit_the_trust_field_and_old_files_read_untrusted() {
    let state = tempfile::tempdir().expect("state");
    let source = tempfile::tempdir().expect("source");
    let bucket = local_bucket(source.path());
    let store = BucketStore::new(state.path());
    store.add("local", &bucket).expect("add");
    let local: Value = serde_json::from_slice(
        &fs::read(state.path().join("buckets/registrations/local.json")).expect("registration"),
    )
    .expect("JSON");
    assert!(local.get("module_trust").is_none());

    let (state, store, _remote, _revision) = fixture();
    let git: Value =
        serde_json::from_slice(&fs::read(registration(state.path())).expect("registration"))
            .expect("JSON");
    assert!(git.get("module_trust").is_none());
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Untrusted
    );
}

#[test]
fn later_sync_keeps_trust_for_the_same_pinned_signer() {
    let (_state, store, _remote, revision) = fixture();
    store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: None,
            },
        )
        .expect("trust");
    let synced = store.sync_git("community", None).expect("resync");
    assert_eq!(synced.signer.key_id, revision.signer.key_id);
    assert_eq!(
        store.module_trust("community").expect("trust state"),
        ModuleTrustState::Trusted
    );
    assert_eq!(store.trusted_module_sources().expect("sources").len(), 1);
}

struct SignedRemote {
    directory: tempfile::TempDir,
    pair: KeyPair,
    public: String,
}

impl SignedRemote {
    fn path(&self) -> &Path {
        self.directory.path()
    }

    fn resign(&self, message: &str) {
        write_signature(self.path(), &self.pair, &self.public);
        commit(self.path(), message);
    }

    fn source(&self) -> GitCatalogSource {
        GitCatalogSource::new(
            self.path().to_string_lossy().into_owned(),
            "fixture".to_owned(),
        )
    }
}

fn signed_remote(layout: impl FnOnce(&Path)) -> SignedRemote {
    let directory = tempfile::tempdir().expect("remote");
    git(directory.path(), &["init", "--initial-branch=fixture"]);
    fs::write(
        directory.path().join("bucket.json"),
        r#"{"schema_version":2,"name":"community","title":"Community"}"#,
    )
    .expect("metadata");
    fs::create_dir_all(directory.path().join("mods/test-game")).expect("mods");
    fs::write(directory.path().join("mods/test-game/mod.json"), package()).expect("package");
    layout(directory.path());
    let pair = KeyPair::generate_unencrypted_keypair().expect("key");
    let public = pair.pk.to_box().expect("public key").into_string();
    write_signature(directory.path(), &pair, &public);
    commit(directory.path(), "fixture");
    SignedRemote {
        directory,
        pair,
        public,
    }
}

#[test]
fn trusted_catalogs_refuse_a_package_manifest_under_games() {
    for (relative, contents) in [
        ("games/test-game/extra.json", package()),
        ("games/test-game/notes.txt", "notes".to_owned()),
        ("games/loose.json", package()),
    ] {
        let remote = signed_remote(|_| {});
        let state = tempfile::tempdir().expect("state");
        let store = BucketStore::new(state.path());
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
    }

    let remote = signed_remote(|_| {});
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(state.path());
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
}

#[test]
fn untrusted_catalogs_ignore_games_and_stay_untrusted() {
    main_bucket::seeded(|| {
        let remote = signed_remote(|root| {
            fs::create_dir_all(root.join("games/test-game")).expect("games");
            fs::write(
            root.join("games/test-game/module.json"),
            r#"{"game_id":"test-game","version":"1","api_version":1,"entrypoint":"module.lua"}"#,
        )
        .expect("module manifest");
            fs::write(root.join("games/test-game/module.lua"), "return {}").expect("module script");
            fs::write(root.join("games/loose.txt"), "ignored").expect("loose entry");
        });
        let state = tempfile::tempdir().expect("state");
        let store = BucketStore::new(state.path());
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

#[test]
fn a_mod_named_module_is_a_package_not_a_game_module() {
    main_bucket::seeded(|| {
        let state = tempfile::tempdir().expect("state");
        let source = tempfile::tempdir().expect("source");
        let bucket = local_bucket(source.path());
        fs::create_dir_all(bucket.join("mods/test-game")).expect("mods");
        fs::write(
            bucket.join("mods/test-game/module.json"),
            package().replace("\"id\":\"mod\"", "\"id\":\"module\""),
        )
        .expect("package named module");
        fs::write(bucket.join("games/test-game/module.lua"), "return {}").expect("ignored script");
        let store = BucketStore::new(state.path());
        store.add("local", &bucket).expect("register");
        let packages = store.packages(Some("test-game")).expect("packages");
        assert!(packages
            .iter()
            .any(|package| package.package.id == "module"));
    });
}

#[test]
fn bucket_listing_reports_module_trust() {
    let temp = tempfile::tempdir().expect("state");
    fs::create_dir_all(temp.path().join("buckets/main")).expect("main");
    fs::write(
        temp.path().join("buckets/main/bucket.json"),
        json!({"schema_version": 2, "name": "main", "title": "Main"}).to_string(),
    )
    .expect("metadata");
    let remote = TestRemote::new();
    modn_core::data_dir::with_data_directory(temp.path(), || {
        let store = BucketStore::new(temp.path());
        let _revision = remote.sync(&store);
        let listed = store.list().expect("list");
        assert!(
            listed
                .iter()
                .find(|bucket| bucket.name == "main")
                .expect("main")
                .module_trusted
        );
        assert!(
            !listed
                .iter()
                .find(|bucket| bucket.name == "community")
                .expect("community")
                .module_trusted
        );
        store
            .trust_modules(
                "community",
                ModuleTrustAck::Acknowledged {
                    expected_fingerprint: Some(&remote.key_id),
                },
            )
            .expect("trust");
        let listed = store.list().expect("list");
        assert!(
            listed
                .iter()
                .find(|bucket| bucket.name == "community")
                .expect("community")
                .module_trusted
        );
    });
}
