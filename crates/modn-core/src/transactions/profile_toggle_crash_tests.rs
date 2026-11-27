//! Recovery of an interrupted profile enable or disable.

use super::install::Fault;
use super::journal::{lock_store, restore_path, temp_path};
use super::profile_toggle::active_profile_id;
use super::profile_toggle_apply::commit_toggle;
use super::restore_tests::Fixture;
use std::fs;

fn crash_toggle(fixture: &mut Fixture, package: &str, enabled: bool) -> String {
    let _guard = lock_store(&fixture.store).expect("lock");
    let result = fixture.store.profile_toggle_with_fault_locked(
        &fixture.game,
        "default",
        package,
        enabled,
        Some(Fault::CrashAfter(1)),
    );
    assert!(result.is_err(), "the injected stop must fail");
    let pending = fixture.store.pending().expect("pending");
    assert_eq!(pending.len(), 1);
    pending[0].id.clone()
}

fn enabled(fixture: &Fixture, package: &str) -> bool {
    fixture
        .store
        .connection
        .query_row(
            "SELECT enabled FROM profile_packages WHERE package_id=?1",
            [package],
            |row| row.get::<_, i64>(0),
        )
        .expect("membership")
        != 0
}

#[test]
fn profile_toggle_crash_before_commit_rolls_back_to_winner() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let id = crash_toggle(&mut fixture, &second.id, false);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert!(restore_path(&fixture.path(), &id, 0).expect("restore").is_file());

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert!(enabled(&fixture, &second.id));
    assert!(!restore_path(&fixture.path(), &id, 0).expect("restore").exists());
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_crash_after_commit_keeps_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let profile = active_profile_id(&fixture.store, &fixture.game).expect("profile");

    let id = crash_toggle(&mut fixture, &second.id, false);
    commit_toggle(&mut fixture.store, &id, &profile, &second.id, false).expect("commit");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert!(!enabled(&fixture, &second.id));
    assert!(!restore_path(&fixture.path(), &id, 0).expect("restore").exists());
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_crash_enable_create_rolls_back_to_absent() {
    let mut fixture = Fixture::new();
    let only = fixture.install("only", b"only-bytes");
    fixture
        .store
        .profile_toggle(&fixture.game, "default", &only.id, false)
        .expect("disable");
    assert!(!fixture.path().exists());

    let id = crash_toggle(&mut fixture, &only.id, true);
    assert_eq!(fixture.bytes(), b"only-bytes");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert!(!fixture.path().exists());
    assert!(!enabled(&fixture, &only.id));
    assert_eq!(fixture.pending(), 0);
    assert!(!restore_path(&fixture.path(), &id, 0).expect("restore").exists());
}

#[test]
fn profile_toggle_crash_enable_replace_rolls_back_to_lower_claimant() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fixture
        .store
        .profile_toggle(&fixture.game, "default", &second.id, false)
        .expect("disable second");
    assert_eq!(fixture.bytes(), b"first-bytes");

    let id = crash_toggle(&mut fixture, &second.id, true);
    assert_eq!(fixture.bytes(), b"second-bytes");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert!(!enabled(&fixture, &second.id));
    assert_eq!(fixture.pending(), 0);
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
}

#[test]
fn profile_toggle_crash_removes_witnesses_after_commit_cleanup() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = crash_toggle(&mut fixture, &second.id, false);
    let witness = temp_path(&fixture.path(), &id, 0).expect("temp");
    assert!(witness.is_file());

    fixture.store.recover().expect("recover");
    assert!(fs::symlink_metadata(&witness).is_err());
}
