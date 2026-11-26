//! Publish order of a Git catalog sync and its staged game modules.

use super::tests::{commit, remote, seed_catalog, source, test_key, write_signature};
use super::BucketStore;
use crate::buckets::ModuleTrustAck;
use crate::game_modules::{ModuleRegistry, ReleaseOutcome};
use crate::manager_roots::ManagerRoots;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

fn main_bucket(state: &Path) {
    let directory = state.join("buckets/main");
    fs::create_dir_all(&directory).expect("main bucket directory");
    fs::write(
        directory.join("bucket.json"),
        r#"{"schema_version":2,"name":"main","title":"Main"}"#,
    )
    .expect("main metadata");
}

fn write_module(catalog: &Path, game: &str, version: &str) {
    let directory = catalog.join("games").join(game);
    fs::create_dir_all(&directory).expect("module directory");
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
    .expect("manifest");
    fs::write(
        directory.join("module.lua"),
        format!("return {{game_id='{game}', api_version=3, version='{version}'}}"),
    )
    .expect("script");
}

fn active_version(state: &Path, game: &str) -> Option<String> {
    ModuleRegistry::new(&ManagerRoots::new(state, None))
        .active(game)
        .expect("active release")
        .map(|release| release.module.version)
}

fn retained_versions(state: &Path, game: &str) -> Vec<String> {
    ModuleRegistry::new(&ManagerRoots::new(state, None))
        .releases(game)
        .expect("releases")
        .into_iter()
        .map(|release| release.module.version)
        .collect()
}

fn registered_commit(store: &BucketStore) -> String {
    store
        .git_revision("community")
        .expect("registration")
        .expect("registered Git catalog")
        .commit
}

fn trust_modules(store: &BucketStore, key_id: &str) {
    store
        .trust_modules(
            "community",
            ModuleTrustAck::Acknowledged {
                expected_fingerprint: Some(key_id),
            },
        )
        .expect("trust modules");
}

fn first_sync(store: &BucketStore, remote: &super::tests::Remote) -> String {
    let first = store
        .sync_git_with_signer("community", Some(source(remote)))
        .expect("first sync");
    assert!(first.modules.imported.is_empty(), "untrusted modules wait");
    first.revision.commit
}

fn second_revision(remote: &super::tests::Remote) {
    write_module(remote.directory.path(), "alpha-game", "2.0.0");
    seed_catalog(remote.directory.path(), "2");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "second");
}

#[test]
fn a_failed_module_commit_keeps_both_sides_on_the_previous_revision() {
    let remote = remote("1", test_key());
    write_module(remote.directory.path(), "alpha-game", "1.0.0");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "first");
    let state = tempfile::tempdir().expect("state");
    main_bucket(state.path());
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    let first = first_sync(&store, &remote);
    trust_modules(&store, &remote.key.public.key_id().to_string());
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("1.0.0")
    );

    second_revision(&remote);
    crate::game_modules::FAIL_NEXT_CATALOG_COMMIT
        .lock()
        .expect("seam")
        .insert(state.path().join("game-modules"));
    let error = store
        .sync_git_with_signer("community", None)
        .expect_err("module commit fails");
    let error = format!("{error:#}");
    assert!(
        error.contains("injected catalog module commit failure"),
        "{error}"
    );

    assert_eq!(registered_commit(&store), first);
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("1.0.0")
    );
    assert_eq!(
        retained_versions(state.path(), "alpha-game"),
        vec!["1.0.0".to_owned()]
    );

    let synced = store
        .sync_git_with_signer("community", None)
        .expect("the next sync repairs both sides");
    assert_ne!(synced.revision.commit, first);
    assert_eq!(synced.modules.imported.len(), 1);
    assert_eq!(synced.modules.imported[0].outcome, ReleaseOutcome::Imported);
    assert_eq!(registered_commit(&store), synced.revision.commit);
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("2.0.0")
    );
}

#[test]
fn a_failed_publish_after_the_module_commit_converges_on_the_next_sync() {
    let remote = remote("1", test_key());
    write_module(remote.directory.path(), "alpha-game", "1.0.0");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "first");
    let state = tempfile::tempdir().expect("state");
    main_bucket(state.path());
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    let first = first_sync(&store, &remote);
    trust_modules(&store, &remote.key.public.key_id().to_string());

    second_revision(&remote);
    store
        .fail_next_registration_write
        .store(true, Ordering::SeqCst);
    let error = store
        .sync_git_with_signer("community", None)
        .expect_err("the registration write fails");
    let error = format!("{error:#}");
    assert!(
        error.contains("injected registration write failure"),
        "{error}"
    );

    assert_eq!(registered_commit(&store), first);
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("2.0.0")
    );

    let synced = store
        .sync_git_with_signer("community", None)
        .expect("the next sync publishes the revision");
    assert_ne!(synced.revision.commit, first);
    assert_eq!(synced.modules.imported.len(), 1);
    assert_eq!(
        synced.modules.imported[0].outcome,
        ReleaseOutcome::Unchanged
    );
    assert_eq!(registered_commit(&store), synced.revision.commit);
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("2.0.0")
    );
}

#[test]
fn an_unchanged_catalog_revision_still_imports_modules() {
    let remote = remote("1", test_key());
    write_module(remote.directory.path(), "alpha-game", "1.0.0");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "first");
    let state = tempfile::tempdir().expect("state");
    main_bucket(state.path());
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    let first = first_sync(&store, &remote);
    trust_modules(&store, &remote.key.public.key_id().to_string());

    fs::remove_dir_all(state.path().join("game-modules")).expect("drop the module registry");
    let synced = store
        .sync_git_with_signer("community", None)
        .expect("an unchanged revision still runs the module step");
    assert_eq!(synced.revision.commit, first);
    assert_eq!(synced.modules.imported.len(), 1);
    assert_eq!(synced.modules.imported[0].outcome, ReleaseOutcome::Imported);
    assert_eq!(
        active_version(state.path(), "alpha-game").as_deref(),
        Some("1.0.0")
    );
}
