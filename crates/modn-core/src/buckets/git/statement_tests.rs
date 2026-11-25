//! Statement-chain rules: only a statement that names the pinned key applies.

use super::tests::{
    commit, registration, remote, source, test_key, write_signature, Remote, TestKey,
};
use super::BucketStore;
use crate::buckets::{
    key_statement_bytes, revocation_statement, rotation_statement, GitCatalogSync, KeyChangeKind,
};
use modn_core::manager_roots::ManagerRoots;
use std::fs;
use std::io::Cursor;
use std::path::Path;

fn sign_statement(key: &TestKey, kind: KeyChangeKind, body: &[u8]) -> String {
    minisign::sign(
        Some(&key.pair.pk),
        &key.pair.sk,
        Cursor::new(key_statement_bytes(kind, "community", body)),
        Some("statement test"),
        Some("statement test signature"),
    )
    .expect("statement signature")
    .into_string()
}

pub(super) fn write_rotation_files(root: &Path, old: &TestKey, new: &TestKey) {
    let body = rotation_statement(&old.public.key_id().to_string(), &new.text)
        .expect("rotation statement");
    fs::write(root.join("rotation.json"), &body).expect("rotation body");
    fs::write(
        root.join("rotation.sig"),
        sign_statement(old, KeyChangeKind::Rotated, &body),
    )
    .expect("rotation signature");
}

fn write_revocation_files(root: &Path, revocation: &TestKey, revoked: &TestKey, next: &TestKey) {
    let body = revocation_statement(&revoked.public.key_id().to_string(), &next.text)
        .expect("revocation statement");
    fs::write(root.join("revocation.json"), &body).expect("revocation body");
    fs::write(
        root.join("revocation.sig"),
        sign_statement(revocation, KeyChangeKind::Revoked, &body),
    )
    .expect("revocation signature");
}

fn seed_remote(version: &str, signer: TestKey) -> Remote {
    remote(version, signer)
}

fn pin_revocation_key(remote: &Remote, revocation: &TestKey) {
    fs::write(
        remote.directory.path().join("revocation.pub"),
        &revocation.text,
    )
    .expect("revocation key");
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "revocation key");
}

fn sync(store: &BucketStore, name: &str) -> anyhow::Result<GitCatalogSync> {
    store.sync_git_with_signer(name, None)
}

fn registration_json(state: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(registration(state)).expect("registration"))
        .expect("registration JSON")
}

fn set_trusted(state: &Path, key_id: &str) {
    let path = registration(state);
    let mut value = registration_json(state);
    value["module_trust"] = serde_json::json!({"signer_key_id": key_id});
    fs::write(
        &path,
        serde_json::to_vec(&value).expect("registration JSON"),
    )
    .expect("write");
}

fn trust_record(state: &Path) -> Option<String> {
    registration_json(state)["module_trust"]["signer_key_id"]
        .as_str()
        .map(str::to_owned)
}

#[test]
fn first_sync_pins_catalog_pub_and_ignores_a_published_rotation() {
    let old = test_key();
    let current = test_key();
    let remote = seed_remote("1", current);
    write_rotation_files(remote.directory.path(), &old, &remote.key);
    write_signature(remote.directory.path(), &remote.key);
    commit(remote.directory.path(), "rotated before first sync");
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));

    let synced = store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("a rotated catalog registers");
    assert!(synced.pinned);
    assert_eq!(
        synced.revision.signer.key_id,
        remote.key.public.key_id().to_string()
    );
    assert_eq!(
        synced.ignored_key_statements,
        vec!["rotation".to_owned()],
        "first sync says the statement was ignored"
    );
    assert_eq!(
        store
            .git_revision("community")
            .expect("revision")
            .expect("registered")
            .signer
            .public_key,
        remote.key.text
    );
}

#[test]
fn first_sync_refuses_a_malformed_published_statement() {
    let old = test_key();
    let current = test_key();
    let remote = seed_remote("1", current.clone());
    let body = br#"{"schema_version":2,"kind":"rotation","from":"0000000000000000","to":"x"}"#;
    fs::write(remote.directory.path().join("rotation.json"), body).expect("body");
    fs::write(
        remote.directory.path().join("rotation.sig"),
        sign_statement(&old, KeyChangeKind::Rotated, body),
    )
    .expect("signature");
    write_signature(remote.directory.path(), &current);
    commit(remote.directory.path(), "unknown version");
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));

    let error = store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect_err("an unknown statement version must be refused")
        .to_string();
    assert!(error.contains("schema_version 2"), "{error}");
    assert!(!registration(state.path()).exists());

    let body = br#"{"schema_version":1,"kind":"rollover","from":"0000000000000000","to":"x"}"#;
    fs::write(remote.directory.path().join("rotation.json"), body).expect("body");
    fs::write(
        remote.directory.path().join("rotation.sig"),
        sign_statement(&old, KeyChangeKind::Rotated, body),
    )
    .expect("signature");
    write_signature(remote.directory.path(), &current);
    commit(remote.directory.path(), "unknown kind");
    let error = store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect_err("an unknown statement kind must be refused")
        .to_string();
    assert!(error.contains("expected \"rotation\""), "{error}");
    assert!(!registration(state.path()).exists());
}

#[test]
fn promised_rotation_is_history_when_the_pinned_key_is_its_successor() {
    let first = test_key();
    let second = test_key();
    let pinned = test_key();
    let remote = seed_remote("1", pinned.clone());
    write_rotation_files(remote.directory.path(), &first, &second);
    write_signature(remote.directory.path(), &pinned);
    commit(remote.directory.path(), "historical rotation");
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");

    let synced = sync(&store, "community").expect("the historical statement is ignored");
    assert_eq!(
        synced.revision.signer.key_id,
        pinned.public.key_id().to_string()
    );
    assert_eq!(synced.ignored_key_statements, vec!["rotation".to_owned()]);
}

#[test]
fn a_revocation_applies_after_an_accepted_rotation_is_kept_published() {
    let old = test_key();
    let pinned = test_key();
    let replacement = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", pinned.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    set_trusted(state.path(), &pinned.public.key_id().to_string());

    write_rotation_files(remote.directory.path(), &old, &pinned);
    write_revocation_files(remote.directory.path(), &revocation, &pinned, &replacement);
    write_signature(remote.directory.path(), &replacement);
    commit(remote.directory.path(), "revocation after rotation");

    let synced = sync(&store, "community").expect("the revocation applies");
    let change = synced.key_change.expect("revocation");
    assert_eq!(change.kind, KeyChangeKind::Revoked);
    assert_eq!(change.from, pinned.public.key_id().to_string());
    assert_eq!(change.to, replacement.public.key_id().to_string());
    assert_eq!(
        synced.revision.signer.key_id,
        replacement.public.key_id().to_string()
    );
    assert_eq!(synced.ignored_key_statements, vec!["rotation".to_owned()]);
    assert_eq!(trust_record(state.path()), None, "revocation clears trust");
}

#[test]
fn a_rotation_applies_after_an_accepted_revocation_is_kept_published() {
    let revoked = test_key();
    let pinned = test_key();
    let replacement = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", pinned.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    set_trusted(state.path(), &pinned.public.key_id().to_string());

    write_revocation_files(remote.directory.path(), &revocation, &revoked, &pinned);
    write_rotation_files(remote.directory.path(), &pinned, &replacement);
    write_signature(remote.directory.path(), &replacement);
    commit(remote.directory.path(), "rotation after revocation");

    let synced = sync(&store, "community").expect("the rotation applies");
    let change = synced.key_change.expect("rotation");
    assert_eq!(change.kind, KeyChangeKind::Rotated);
    assert_eq!(change.from, pinned.public.key_id().to_string());
    assert_eq!(change.to, replacement.public.key_id().to_string());
    assert_eq!(
        synced.revision.signer.key_id,
        replacement.public.key_id().to_string()
    );
    assert_eq!(synced.ignored_key_statements, vec!["revocation".to_owned()]);
    assert_eq!(
        trust_record(state.path()),
        Some(replacement.public.key_id().to_string()),
        "rotation moves trust to the new signer"
    );
}

#[test]
fn a_rotation_back_to_a_previous_key_keeps_the_pinned_key_current() {
    let original = test_key();
    let middle = test_key();
    let remote = seed_remote("1", original.clone());
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");

    write_rotation_files(remote.directory.path(), &original, &middle);
    write_signature(remote.directory.path(), &middle);
    commit(remote.directory.path(), "rotate to the middle key");
    let rotated = sync(&store, "community").expect("rotation applies");
    assert_eq!(
        rotated.revision.signer.key_id,
        middle.public.key_id().to_string()
    );

    write_rotation_files(remote.directory.path(), &middle, &original);
    write_signature(remote.directory.path(), &original);
    commit(remote.directory.path(), "rotate back");
    let back = sync(&store, "community").expect("a rotation back to a previous key applies");
    assert_eq!(
        back.revision.signer.key_id,
        original.public.key_id().to_string()
    );
    assert_eq!(
        back.revision.signer.previous_signers,
        vec![middle.public.key_id().to_string()],
        "the old current key is retired, the new current key is not"
    );
    assert!(back.revision.signer.revoked_signers.is_empty());

    let again = sync(&store, "community").expect("the retained rotation is history");
    assert_eq!(
        again.revision.signer.key_id,
        original.public.key_id().to_string(),
        "the pinned key is never retired by its own rotation history"
    );
}

#[test]
fn a_rotation_naming_a_revoked_key_is_refused() {
    let pinned = test_key();
    let replacement = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", pinned.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    write_revocation_files(remote.directory.path(), &revocation, &pinned, &replacement);
    write_signature(remote.directory.path(), &replacement);
    commit(remote.directory.path(), "revoke the pinned key");
    sync(&store, "community").expect("revocation applies");
    let before = fs::read(registration(state.path())).expect("registration");

    write_rotation_files(remote.directory.path(), &replacement, &pinned);
    write_signature(remote.directory.path(), &pinned);
    commit(remote.directory.path(), "rotate back to the revoked key");
    let error = sync(&store, "community")
        .expect_err("a rotation cannot re-authorize a revoked key")
        .to_string();
    assert!(
        error.contains("was revoked by the offline revocation key"),
        "{error}"
    );
    assert_eq!(
        fs::read(registration(state.path())).expect("registration"),
        before
    );
}

#[test]
fn a_revocation_can_reauthorize_a_previous_key() {
    let original = test_key();
    let current = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", original.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    write_rotation_files(remote.directory.path(), &original, &current);
    write_signature(remote.directory.path(), &current);
    commit(remote.directory.path(), "rotate away");
    sync(&store, "community").expect("rotation applies");

    write_revocation_files(remote.directory.path(), &revocation, &current, &original);
    write_signature(remote.directory.path(), &original);
    commit(remote.directory.path(), "revoke the current key");
    let reauthorized = sync(&store, "community").expect("a revocation may re-authorize a key");
    assert_eq!(
        reauthorized.revision.signer.key_id,
        original.public.key_id().to_string()
    );
    assert!(
        reauthorized.revision.signer.previous_signers.is_empty(),
        "the re-authorized key is current, not previous"
    );
    assert_eq!(
        reauthorized.revision.signer.revoked_signers,
        vec![current.public.key_id().to_string()]
    );

    let again = sync(&store, "community").expect("the retained statements are history");
    assert_eq!(
        again.revision.signer.key_id,
        original.public.key_id().to_string()
    );
}

#[test]
fn a_rotated_away_key_refuses_an_unaccompanied_revision() {
    let original = test_key();
    let current = test_key();
    let remote = seed_remote("1", original.clone());
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    write_rotation_files(remote.directory.path(), &original, &current);
    write_signature(remote.directory.path(), &current);
    commit(remote.directory.path(), "rotate away");
    sync(&store, "community").expect("rotation applies");

    fs::remove_file(remote.directory.path().join("rotation.json")).expect("remove body");
    fs::remove_file(remote.directory.path().join("rotation.sig")).expect("remove signature");
    write_signature(remote.directory.path(), &original);
    commit(remote.directory.path(), "republish with the retired key");
    let error = sync(&store, "community")
        .expect_err("a rotated-away key stays refused")
        .to_string();
    assert!(error.contains("was rotated away"), "{error}");
}

#[test]
fn two_statements_that_apply_to_the_pinned_key_are_refused() {
    let pinned = test_key();
    let replacement = test_key();
    let revoked_to = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", pinned.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");
    let before = fs::read(registration(state.path())).expect("registration");

    write_rotation_files(remote.directory.path(), &pinned, &replacement);
    write_revocation_files(remote.directory.path(), &revocation, &pinned, &revoked_to);
    write_signature(remote.directory.path(), &replacement);
    commit(remote.directory.path(), "ambiguous change");

    let error = sync(&store, "community")
        .expect_err("two applicable statements are ambiguous")
        .to_string();
    assert!(
        error.contains("both rotation.json and revocation.json"),
        "{error}"
    );
    assert!(error.contains("apply to the pinned signing key"), "{error}");
    assert_eq!(
        fs::read(registration(state.path())).expect("registration"),
        before
    );
}

#[test]
fn a_two_hop_change_from_the_pinned_key_is_refused() {
    let pinned = test_key();
    let middle = test_key();
    let final_key = test_key();
    let revocation = test_key();
    let remote = seed_remote("1", pinned.clone());
    pin_revocation_key(&remote, &revocation);
    let state = tempfile::tempdir().expect("state");
    let store = BucketStore::new(&ManagerRoots::new(state.path(), None));
    store
        .sync_git_with_signer("community", Some(source(&remote)))
        .expect("first sync");

    write_rotation_files(remote.directory.path(), &pinned, &middle);
    write_revocation_files(remote.directory.path(), &revocation, &middle, &final_key);
    write_signature(remote.directory.path(), &final_key);
    commit(remote.directory.path(), "two hops");

    let error = sync(&store, "community")
        .expect_err("one hop per sync")
        .to_string();
    assert!(
        error.contains("more than one signing-key change from the pinned key"),
        "{error}"
    );
    assert!(error.contains("intermediate revision"), "{error}");
}
