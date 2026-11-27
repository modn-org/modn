//! Recovery of an interrupted profile apply.
//!
//! The apply journal reuses the profile toggle witnesses: the old winner's
//! bytes in `.tmp`, the new winner's bytes in `.restore`, and the old winner's
//! digest in `pending_files.blake3`. A pending journal rolls back; a committed
//! journal only removes witnesses.

use super::install::Fault;
use super::journal::{lock_store, restore_path, temp_path};
use super::profile_toggle::active_profile_id;
use super::profile_toggle_apply::commit_apply;
use super::restore_tests::{Fixture, SHARED};
use crate::profiles::Placement;
use std::fs;

fn crash_apply(fixture: &mut Fixture, package: &str) -> String {
    let _guard = lock_store(&fixture.store).expect("lock");
    let result = fixture.store.set_priority_applied_with_fault_locked(
        &fixture.game,
        "default",
        package,
        &Placement::Rank(1),
        true,
        Some(Fault::CrashAfter(1)),
    );
    assert!(result.is_err(), "the injected stop must fail");
    drop(_guard);
    let pending = fixture.store.pending().expect("pending");
    assert_eq!(pending.len(), 1);
    pending[0].id.clone()
}

fn rank(fixture: &Fixture, package: &str) -> usize {
    let profile = crate::profiles::list(&fixture.store, &fixture.game)
        .expect("profiles")
        .into_iter()
        .find(|profile| profile.name == "default")
        .expect("default profile");
    profile
        .packages
        .iter()
        .position(|entry| entry.package_id == package)
        .expect("membership")
        + 1
}

#[test]
fn profile_apply_crash_after_placement_restores_old_winner() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let id = crash_apply(&mut fixture, &first.id);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert!(restore_path(&fixture.path(), &id, 0)
        .expect("restore")
        .is_file());
    assert!(temp_path(&fixture.path(), &id, 0).expect("temp").is_file());

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(rank(&fixture, &second.id), 1);
    assert_eq!(rank(&fixture, &first.id), 2);
    assert!(!restore_path(&fixture.path(), &id, 0)
        .expect("restore")
        .exists());
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_crash_after_commit_keeps_new_winner_and_ranks() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let profile = active_profile_id(&fixture.store, &fixture.game).expect("profile");

    let id = crash_apply(&mut fixture, &first.id);
    let ranks = vec![first.id.clone(), second.id.clone()];
    commit_apply(&mut fixture.store, &id, &profile, Some(&ranks)).expect("commit");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert_eq!(rank(&fixture, &first.id), 1);
    assert_eq!(fixture.pending(), 0);
    assert!(!restore_path(&fixture.path(), &id, 0)
        .expect("restore")
        .exists());
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
}

#[test]
fn profile_apply_crash_create_rolls_back_to_absent() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = "txn-apply-created";
    let game = fixture.game.to_string_lossy().into_owned();
    let shared = fixture.game.join(SHARED);
    fs::remove_file(&shared).expect("remove managed file");
    let connection = &fixture.store.connection;
    connection
        .execute(
            "INSERT INTO pending_transactions
             (id, action, status, game_path, package_id, archive_blake3)
             VALUES (?1, 'profile_apply', 'pending', ?2, '', NULL)",
            rusqlite::params![id, game],
        )
        .expect("pending transaction");
    connection
        .execute(
            "INSERT INTO pending_files
             (transaction_id, ord, root, relative_path, blake3, bytes)
             VALUES (?1, 0, 'game', ?2, '', 0)",
            rusqlite::params![id, SHARED],
        )
        .expect("pending file");
    connection
        .execute(
            "INSERT INTO pending_restores
             (transaction_id, ord, package_id, archive_entry, blake3, bytes)
             VALUES (?1, 0, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                id,
                first.id,
                SHARED,
                blake3::hash(b"first-bytes").to_hex().to_string(),
                b"first-bytes".len() as i64
            ],
        )
        .expect("pending restore");
    let restore = restore_path(&shared, id, 0).expect("restore");
    fs::write(&restore, b"first-bytes").expect("restore witness");
    fs::write(&shared, b"first-bytes").expect("interrupted placement");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert!(!shared.exists());
    assert!(!restore.exists());
    assert_eq!(rank(&fixture, &second.id), 1);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_crash_committed_cleanup_removes_witnesses() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = crash_apply(&mut fixture, &first.id);
    let profile = active_profile_id(&fixture.store, &fixture.game).expect("profile");
    let ranks = vec![first.id.clone(), second.id.clone()];

    commit_apply(&mut fixture.store, &id, &profile, Some(&ranks)).expect("commit");
    fixture.store.recover().expect("recover");

    assert_eq!(fixture.pending(), 0);
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
    assert!(!restore_path(&fixture.path(), &id, 0)
        .expect("restore")
        .exists());
    assert_eq!(fixture.bytes(), b"first-bytes");
}
