#[path = "support/catalog_git.rs"]
mod support;

#[path = "support/main_bucket.rs"]
mod main_bucket;
use minisign::KeyPair;
use modn_core::buckets::{
    BucketStore, CatalogPublicKey, GitCatalogRevision, GitCatalogSource, ModuleTrustAck,
    ModuleTrustState,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use support::{commit, git, package, registration, write_signature};

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
    let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
        state.path(),
        None,
    ));
    let revision = remote.sync(&store);
    (state, store, remote, revision)
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
    let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
        state.path(),
        None,
    ));
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
    assert!(
        error.contains("local folder bucket local cannot supply game modules"),
        "{error}"
    );
    assert!(
        error.contains(
            "only the official main bucket and signed Git catalogs the user trusts for modules supply game modules"
        ),
        "{error}"
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
    let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
        state.path(),
        None,
    ));
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
    let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
        state.path(),
        None,
    ));
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
    let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
        state.path(),
        None,
    ));
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

#[test]
fn a_mod_named_module_is_a_package_not_a_game_module() {
    main_bucket::seeded_roots(|roots| {
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
        let store = BucketStore::new(&main_bucket::roots_with_profile(roots.data(), state.path()));
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
    {
        let store = BucketStore::new(&modn_core::manager_roots::ManagerRoots::new(
            temp.path(),
            None,
        ));
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
    }
}
