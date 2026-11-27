//! Enable and disable of profile memberships with real archive restores.
//!
//! Each test uses the disposable game tree and seeded archive cache of the
//! uninstall restore tests. A winning package's bytes come back from the
//! verified archive cache; a toggled package keeps its claims and priority.

use super::profile_toggle::{ProfileToggle, ToggleAction};
use super::restore_tests::{Fixture, SHARED};
use crate::error::{code_of, ErrorCode};
use crate::profiles;
use std::fs;

fn toggle(fixture: &mut Fixture, package: &str, enabled: bool) -> anyhow::Result<ProfileToggle> {
    fixture
        .store
        .profile_toggle(&fixture.game, "default", package, enabled)
}

fn toggle_in(
    fixture: &mut Fixture,
    profile: &str,
    package: &str,
    enabled: bool,
) -> anyhow::Result<ProfileToggle> {
    fixture
        .store
        .profile_toggle(&fixture.game, profile, package, enabled)
}

fn enabled(fixture: &Fixture, profile: &str, package: &str) -> bool {
    fixture
        .store
        .connection
        .query_row(
            "SELECT member.enabled FROM profile_packages member
             JOIN profiles profile ON profile.id = member.profile_id
             WHERE profile.name=?1 AND member.package_id=?2",
            rusqlite::params![profile, package],
            |row| row.get::<_, i64>(0),
        )
        .expect("membership")
        != 0
}

fn count(fixture: &Fixture, table: &str) -> i64 {
    fixture
        .store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
}

#[test]
fn profile_toggle_disable_winner_restores_next_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let report = toggle(&mut fixture, &second.id, false).expect("disable winner");

    assert!(report.active);
    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].action, ToggleAction::Restore);
    assert_eq!(report.changes[0].from.as_deref(), Some(second.id.as_str()));
    assert_eq!(report.changes[0].to.as_deref(), Some(first.id.as_str()));
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert!(!enabled(&fixture, "default", &second.id));
    assert_eq!(count(&fixture, "file_claims"), 2);
    assert_eq!(count(&fixture, "managed_files"), 1);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_disable_shadowed_package_changes_no_file() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let report = toggle(&mut fixture, &first.id, false).expect("disable shadowed");

    assert!(report.changes.is_empty());
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert!(!enabled(&fixture, "default", &first.id));
    assert_eq!(count(&fixture, "file_claims"), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_disable_sole_claimant_removes_file() {
    let mut fixture = Fixture::new();
    let only = fixture.install("only", b"only-bytes");

    let report = toggle(&mut fixture, &only.id, false).expect("disable sole claimant");

    assert_eq!(report.changes[0].action, ToggleAction::Remove);
    assert_eq!(report.changes[0].to, None);
    assert!(!fixture.path().exists());
    assert_eq!(count(&fixture, "managed_files"), 0);
    assert_eq!(count(&fixture, "file_claims"), 1);
    assert!(!enabled(&fixture, "default", &only.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_enable_restores_winner_over_lower_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    toggle(&mut fixture, &second.id, false).expect("disable second");
    assert_eq!(fixture.bytes(), b"first-bytes");

    let report = toggle(&mut fixture, &second.id, true).expect("enable second");

    assert_eq!(report.changes[0].action, ToggleAction::Replace);
    assert_eq!(report.changes[0].from.as_deref(), Some(first.id.as_str()));
    assert_eq!(report.changes[0].to.as_deref(), Some(second.id.as_str()));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert!(enabled(&fixture, "default", &second.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_enable_creates_file_nobody_wins() {
    let mut fixture = Fixture::new();
    let only = fixture.install("only", b"only-bytes");
    toggle(&mut fixture, &only.id, false).expect("disable only");
    assert!(!fixture.path().exists());

    let report = toggle(&mut fixture, &only.id, true).expect("enable only");

    assert_eq!(report.changes[0].action, ToggleAction::Create);
    assert_eq!(report.changes[0].from, None);
    assert_eq!(report.changes[0].to.as_deref(), Some(only.id.as_str()));
    assert_eq!(fixture.bytes(), b"only-bytes");
    assert_eq!(fixture.winner(), Some(only.id.clone()));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_chain_of_three_restores_in_priority_order() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");

    toggle(&mut fixture, &third.id, false).expect("disable third");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));

    toggle(&mut fixture, &second.id, false).expect("disable second");
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));

    toggle(&mut fixture, &third.id, true).expect("enable third");
    assert_eq!(fixture.bytes(), b"third-bytes");
    assert_eq!(fixture.winner(), Some(third.id.clone()));
}

#[test]
fn profile_toggle_disabled_package_is_skipped_by_uninstall_restore() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");
    toggle(&mut fixture, &first.id, false).expect("disable shadowed first");

    let game = fixture.game.clone();
    fixture
        .store
        .uninstall(&game, &third.id)
        .expect("uninstall third");

    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert!(!enabled(&fixture, "default", &first.id));
}

#[test]
fn profile_toggle_missing_archive_refuses_without_change() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fs::remove_dir_all(fixture.store.state_root.join("cache")).expect("remove cache");

    let error = toggle(&mut fixture, &second.id, false).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    let message = format!("{error:#}");
    assert!(message.starts_with("PROFILE_ARCHIVE_MISSING:"), "{message}");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert!(enabled(&fixture, "default", &second.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_archive_digest_mismatch_refuses() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fixture
        .store
        .connection
        .execute(
            "UPDATE file_claims SET blake3=?1 WHERE relative_path=?2 AND package_id<>?3",
            rusqlite::params![
                blake3::hash(b"wrong").to_hex().to_string(),
                SHARED,
                second.id
            ],
        )
        .expect("tamper claim");

    let error = toggle(&mut fixture, &second.id, false).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert!(enabled(&fixture, "default", &second.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_drifted_winner_refuses_with_profile_drift() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fs::write(fixture.path(), b"user-edit").expect("edit winner");

    let error = toggle(&mut fixture, &second.id, false).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    let message = format!("{error:#}");
    assert!(message.starts_with("PROFILE_DRIFT:"), "{message}");
    assert!(message.contains("modn doctor"), "{message}");
    assert_eq!(fixture.bytes(), b"user-edit");
    assert!(enabled(&fixture, "default", &second.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_unmanaged_file_collision_refuses_untouched() {
    let mut fixture = Fixture::new();
    let only = fixture.install("only", b"only-bytes");
    toggle(&mut fixture, &only.id, false).expect("disable only");
    fs::write(fixture.path(), b"user-bytes").expect("unmanaged collision");

    let error = toggle(&mut fixture, &only.id, true).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    assert_eq!(fixture.bytes(), b"user-bytes");
    assert!(!enabled(&fixture, "default", &only.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_inactive_profile_flips_state_only() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    profiles::create(&mut fixture.store, &fixture.game, "spare", Some("default"))
        .expect("create spare");

    let report = toggle_in(&mut fixture, "spare", &second.id, false).expect("disable in spare");

    assert!(!report.active);
    assert!(report.changes.is_empty());
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert!(enabled(&fixture, "default", &second.id));
    assert!(!enabled(&fixture, "spare", &second.id));
    assert!(enabled(&fixture, "spare", &first.id));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_toggle_repeats_are_noop_successes() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    toggle(&mut fixture, &second.id, false).expect("disable");
    let repeated = toggle(&mut fixture, &second.id, false).expect("disable again");
    assert!(repeated.changes.is_empty());
    assert_eq!(fixture.bytes(), b"first-bytes");
    let restored = toggle(&mut fixture, &second.id, true).expect("enable");
    assert_eq!(restored.changes.len(), 1);
    let repeated = toggle(&mut fixture, &second.id, true).expect("enable again");
    assert!(repeated.changes.is_empty());
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.pending(), 0);
}
