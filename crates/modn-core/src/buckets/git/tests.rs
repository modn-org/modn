use super::{git, BucketStore, GitCatalogSource};
use crate::buckets::{catalog_statement, CatalogPublicKey};
use minisign::KeyPair;
use std::fs;
use std::io::Cursor;
use std::path::Path;

fn commit(root: &Path, title: &str) -> String {
    git(root, &["add", "."]).unwrap();
    git(
        root,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            title,
        ],
    )
    .unwrap();
    git(root, &["rev-parse", "HEAD"]).unwrap()
}

fn package(version: &str) -> String {
    serde_json::json!({"schema_version":1,"id":"mod","game":"test-game","name":"Mod",
        "version":version,"archive_format":"zip","sha256":"0".repeat(64),
        "source":{"kind":"manual","filename":"mod.zip"}})
    .to_string()
}

fn seed_catalog(root: &Path, version: &str) {
    fs::write(
        root.join("bucket.json"),
        r#"{"schema_version":1,"name":"community","title":"Community"}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("games/test-game")).unwrap();
    fs::write(root.join("games/test-game/mod.json"), package(version)).unwrap();
}

struct TestKey {
    pair: KeyPair,
    public: CatalogPublicKey,
    text: String,
}

fn test_key() -> TestKey {
    let pair = KeyPair::generate_unencrypted_keypair().unwrap();
    let text = pair.pk.to_box().unwrap().into_string();
    let public = CatalogPublicKey::parse(&text).unwrap();
    TestKey { pair, public, text }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".git" {
            continue;
        }
        let target = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn write_signature(root: &Path, key: &TestKey) {
    let staging = tempfile::tempdir().unwrap();
    copy_tree(root, staging.path());
    let statement = catalog_statement(staging.path()).unwrap();
    let signature = minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(statement.as_bytes()),
        Some("catalog test"),
        Some("catalog test signature"),
    )
    .unwrap()
    .into_string();
    fs::write(root.join("catalog.pub"), &key.text).unwrap();
    fs::write(root.join("catalog.sig"), signature).unwrap();
}

struct Remote {
    directory: tempfile::TempDir,
    key: TestKey,
}

fn remote(version: &str, key: TestKey) -> Remote {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), &["init", "--initial-branch=fixture"]).unwrap();
    seed_catalog(directory.path(), version);
    write_signature(directory.path(), &key);
    commit(directory.path(), "first");
    Remote { directory, key }
}

fn source(remote: &Remote) -> GitCatalogSource {
    GitCatalogSource {
        remote: remote.directory.path().to_string_lossy().into_owned(),
        reference: "fixture".into(),
    }
}

fn registration(state: &Path) -> std::path::PathBuf {
    state.join("buckets/registrations/community.json")
}

fn published(state: &Path) -> std::path::PathBuf {
    let bytes = fs::read(registration(state)).unwrap();
    let pointer: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    std::path::PathBuf::from(pointer["path"].as_str().unwrap())
}

#[test]
fn signed_sync_pins_signer_and_tracks_later_pinned_revisions() {
    let remote = remote("1", test_key());
    let key_id = remote.key.public.key_id().to_string();
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let first_source = source(&remote);

    let first = store
        .sync_git("community", Some(first_source.clone()))
        .unwrap();
    assert!(first.pinned);
    assert_eq!(first.revision.signer.key_id, key_id);
    assert_eq!(first.revision.signer.public_key, remote.key.text);
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        first.revision.commit
    );
    let pointer: serde_json::Value =
        serde_json::from_slice(&fs::read(registration(state.path())).unwrap()).unwrap();
    assert_eq!(pointer["git"]["signer"]["key_id"], key_id);
    assert_eq!(pointer["git"]["signer"]["public_key"], remote.key.text);

    seed_catalog(remote.directory.path(), "2");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "second");
    fs::remove_file(remote.directory.path().join("catalog.pub")).unwrap();
    let second = {
        git(remote.directory.path(), &["add", "."]).unwrap();
        git(
            remote.directory.path(),
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "remove public key",
            ],
        )
        .unwrap();
        git(remote.directory.path(), &["rev-parse", "HEAD"]).unwrap()
    };

    let synced = store.sync_git("community", None).unwrap();
    assert!(!synced.pinned);
    assert_eq!(synced.revision.commit, second);
    assert_ne!(synced.revision.commit, first.revision.commit);
    assert_eq!(synced.revision.signer.key_id, key_id);
    assert!(published(state.path())
        .join("games/test-game/mod.json")
        .is_file());
    assert!(
        fs::read_to_string(published(state.path()).join("games/test-game/mod.json"))
            .unwrap()
            .contains("\"2\"")
    );
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().signer,
        first.revision.signer
    );
}

#[test]
fn changed_signer_key_is_refused_and_preserves_the_published_copy() {
    let remote = remote("1", test_key());
    let new_key = test_key();
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let first = store.sync_git("community", Some(source(&remote))).unwrap();
    let before_registration = fs::read(registration(state.path())).unwrap();
    let before_package =
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap();

    seed_catalog(remote.directory.path(), "2");
    write_signature(remote.directory.path(), &new_key);
    commit(remote.directory.path(), "new signer");

    let error = store.sync_git("community", None).unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "Signer key changed from {} to {} for bucket community.",
            remote.key.public.key_id(),
            new_key.public.key_id()
        )),
        "{error}"
    );
    assert!(error.contains("#122"), "{error}");
    assert!(
        error.contains("remove the bucket and sync it again to trust the new key."),
        "{error}"
    );
    assert_eq!(
        fs::read(registration(state.path())).unwrap(),
        before_registration
    );
    assert_eq!(
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap(),
        before_package
    );
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        first.revision.commit
    );
}

#[test]
fn unsigned_first_sync_is_refused_without_a_registration() {
    let remote = remote("1", test_key());
    fs::remove_file(remote.directory.path().join("catalog.sig")).unwrap();
    fs::remove_file(remote.directory.path().join("catalog.pub")).unwrap();
    commit(remote.directory.path(), "unsigned");
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let error = store
        .sync_git("community", Some(source(&remote)))
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "Git catalog community at fixture is unsigned; Modn refuses unsigned catalogs."
    );
    assert!(!registration(state.path()).exists());
    assert!(store.git_revision("community").unwrap().is_none());
}

#[test]
fn first_sync_requires_both_signature_and_public_key() {
    let remote = remote("1", test_key());
    fs::remove_file(remote.directory.path().join("catalog.sig")).unwrap();
    commit(remote.directory.path(), "public key only");
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let error = store
        .sync_git("community", Some(source(&remote)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("is unsigned"), "{error}");
    assert!(!registration(state.path()).exists());

    write_signature(remote.directory.path(), &remote.key);
    fs::remove_file(remote.directory.path().join("catalog.pub")).unwrap();
    commit(remote.directory.path(), "signature only");
    let error = store
        .sync_git("community", Some(source(&remote)))
        .unwrap_err()
        .to_string();
    assert!(error.contains("is unsigned"), "{error}");
    assert!(!registration(state.path()).exists());
}

#[test]
fn invalid_signature_is_refused_at_first_and_later_sync() {
    let tampered = remote("1", test_key());
    fs::write(
        tampered.directory.path().join("games/test-game/mod.json"),
        package("tampered"),
    )
    .unwrap();
    commit(tampered.directory.path(), "tampered before first sync");
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let error = store
        .sync_git("community", Some(source(&tampered)))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("catalog signature verification failed"),
        "{error}"
    );
    assert!(!registration(state.path()).exists());

    let valid = remote("1", test_key());
    let first = store.sync_git("community", Some(source(&valid))).unwrap();
    let before_registration = fs::read(registration(state.path())).unwrap();
    let before_package =
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap();
    fs::write(
        valid.directory.path().join("games/test-game/mod.json"),
        package("tampered"),
    )
    .unwrap();
    commit(valid.directory.path(), "tampered after first sync");
    let error = store.sync_git("community", None).unwrap_err().to_string();
    assert!(
        error.contains("catalog signature verification failed"),
        "{error}"
    );
    assert_eq!(
        fs::read(registration(state.path())).unwrap(),
        before_registration
    );
    assert_eq!(
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap(),
        before_package
    );
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        first.revision.commit
    );
}

#[test]
fn unsigned_later_revision_is_refused_and_preserves_the_published_copy() {
    let remote = remote("1", test_key());
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let first = store.sync_git("community", Some(source(&remote))).unwrap();
    let before_registration = fs::read(registration(state.path())).unwrap();
    let before_package =
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap();

    seed_catalog(remote.directory.path(), "2");
    fs::remove_file(remote.directory.path().join("catalog.sig")).unwrap();
    commit(remote.directory.path(), "missing signature");
    let error = store.sync_git("community", None).unwrap_err().to_string();
    assert!(error.contains("is unsigned"), "{error}");
    assert_eq!(
        fs::read(registration(state.path())).unwrap(),
        before_registration
    );
    assert_eq!(
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap(),
        before_package
    );

    fs::remove_file(remote.directory.path().join("catalog.pub")).unwrap();
    commit(remote.directory.path(), "missing public key");
    let error = store.sync_git("community", None).unwrap_err().to_string();
    assert!(error.contains("is unsigned"), "{error}");
    assert_eq!(
        fs::read(registration(state.path())).unwrap(),
        before_registration
    );
    assert_eq!(
        fs::read(published(state.path()).join("games/test-game/mod.json")).unwrap(),
        before_package
    );
    assert_eq!(
        store.git_revision("community").unwrap().unwrap().commit,
        first.revision.commit
    );
}

#[test]
fn registration_without_a_signer_pin_is_refused() {
    let remote = remote("1", test_key());
    let state = tempfile::tempdir().unwrap();
    let store = BucketStore::new(state.path());
    let first_source = source(&remote);
    store
        .sync_git("community", Some(first_source.clone()))
        .unwrap();
    let path = registration(state.path());
    let mut pointer: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    pointer["git"].as_object_mut().unwrap().remove("signer");
    fs::write(&path, serde_json::to_vec(&pointer).unwrap()).unwrap();
    let message =
        "Bucket community was registered before catalog signing; remove it and sync it again.";

    assert_eq!(
        store.git_revision("community").unwrap_err().to_string(),
        message
    );
    assert_eq!(
        store.sync_git("community", None).unwrap_err().to_string(),
        message
    );

    store.remove("community").unwrap();
    let synced = store.sync_git("community", Some(first_source)).unwrap();
    assert!(synced.pinned);
}
