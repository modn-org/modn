//! Applying a profile's priority order to the game files.

use super::install::Fault;
use super::journal::lock_store;
use super::restore_tests::{Fixture, SHARED, UNMANAGED};
use crate::error::{code_of, ErrorCode};
use crate::profiles::{self, Placement};
use crate::transactions::ToggleAction;
use std::fs;

fn move_to(fixture: &mut Fixture, package: &str, rank: i64) -> crate::transactions::PriorityReport {
    fixture
        .store
        .move_priority(&fixture.game, "default", package, &Placement::Rank(rank))
        .expect("move priority")
}

fn apply(fixture: &mut Fixture) -> crate::transactions::ProfileApply {
    fixture
        .store
        .apply_profile(&fixture.game, Some("default"))
        .expect("apply profile")
}

fn rank(fixture: &Fixture, package: &str) -> usize {
    let profile = profiles::list(&fixture.store, &fixture.game)
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

fn apply_with_fault(
    fixture: &mut Fixture,
    package: &str,
    placement: Placement,
    fault: Fault,
) -> anyhow::Result<crate::transactions::PriorityReport> {
    let _guard = lock_store(&fixture.store).expect("lock");
    fixture.store.set_priority_applied_with_fault_locked(
        &fixture.game,
        "default",
        package,
        &placement,
        true,
        Some(fault),
    )
}

#[test]
fn profile_apply_swap_materializes_new_winner() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    assert_eq!(fixture.bytes(), b"second-bytes");

    move_to(&mut fixture, &second.id, 2);
    assert_eq!(fixture.bytes(), b"second-bytes");
    let report = apply(&mut fixture);

    assert_eq!(report.changes.len(), 1);
    assert_eq!(report.changes[0].path, SHARED);
    assert_eq!(report.changes[0].action, ToggleAction::Replace);
    assert_eq!(report.changes[0].from.as_deref(), Some(second.id.as_str()));
    assert_eq!(report.changes[0].to.as_deref(), Some(first.id.as_str()));
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert_eq!(rank(&fixture, &first.id), 1);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_three_way_reorder_materializes_each_winner() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");

    move_to(&mut fixture, &first.id, 1);
    apply(&mut fixture);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));

    move_to(&mut fixture, &second.id, 1);
    apply(&mut fixture);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));

    move_to(&mut fixture, &third.id, 1);
    apply(&mut fixture);
    assert_eq!(fixture.bytes(), b"third-bytes");
    assert_eq!(fixture.winner(), Some(third.id.clone()));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_already_consistent_is_a_noop() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let before = fs::read(fixture.path()).expect("bytes");

    let report = apply(&mut fixture);

    assert!(report.changes.is_empty());
    assert_eq!(fs::read(fixture.path()).expect("bytes"), before);
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_ignores_disabled_packages() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");
    fixture
        .store
        .profile_toggle(&fixture.game, "default", &third.id, false)
        .expect("disable third");
    assert_eq!(fixture.bytes(), b"second-bytes");

    move_to(&mut fixture, &third.id, 1);
    let report = apply(&mut fixture);

    assert!(report.changes.is_empty());
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
}

#[test]
fn profile_apply_missing_archive_refuses_and_keeps_ranks() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fs::remove_dir_all(fixture.store.state_root.join("cache")).expect("remove cache");

    let error = fixture
        .store
        .set_priority_applied(
            &fixture.game,
            "default",
            &first.id,
            &Placement::Rank(1),
            true,
        )
        .expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert!(format!("{error:#}").contains("modn cache show"));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(rank(&fixture, &second.id), 1);
    assert_eq!(rank(&fixture, &first.id), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_claim_digest_mismatch_refuses() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fixture
        .store
        .connection
        .execute(
            "UPDATE file_claims SET blake3=?1 WHERE relative_path=?2 AND package_id=?3",
            rusqlite::params![
                blake3::hash(b"wrong").to_hex().to_string(),
                SHARED,
                first.id
            ],
        )
        .expect("tamper claim");

    let error = fixture
        .store
        .set_priority_applied(
            &fixture.game,
            "default",
            &first.id,
            &Placement::Rank(1),
            true,
        )
        .expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(rank(&fixture, &second.id), 1);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_drifted_winner_refuses() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    fixture.install("second", b"second-bytes");
    fs::write(fixture.path(), b"user-edit").expect("edit winner");

    let error = fixture
        .store
        .set_priority_applied(
            &fixture.game,
            "default",
            &first.id,
            &Placement::Rank(1),
            true,
        )
        .expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    assert!(format!("{error:#}").contains("modn doctor"));
    assert_eq!(fixture.bytes(), b"user-edit");
    assert_eq!(rank(&fixture, &first.id), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_leaves_unmanaged_files_untouched() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    fixture.install("second", b"second-bytes");

    move_to(&mut fixture, &first.id, 1);
    apply(&mut fixture);

    assert_eq!(
        fs::read(fixture.game.join(UNMANAGED)).expect("unmanaged"),
        b"user-bytes"
    );
}

#[test]
fn profile_apply_inactive_profile_refuses() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    profiles::create(&mut fixture.store, &fixture.game, "spare", Some("default"))
        .expect("create spare");

    let error = fixture
        .store
        .apply_profile(&fixture.game, Some("spare"))
        .expect_err("refuse inactive");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileFailed));
    assert!(format!("{error:#}").contains("not active"));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
}

#[test]
fn profile_apply_ranks_only_change_leaves_files_and_winners() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let report = move_to(&mut fixture, &first.id, 1);

    assert_eq!(report.rank, 1);
    assert!(!report.applied);
    assert!(report.changes.is_empty());
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(rank(&fixture, &first.id), 1);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_rank_and_winners_commit_together() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    let result = apply_with_fault(
        &mut fixture,
        &first.id,
        Placement::Rank(1),
        Fault::CrashAfter(1),
    );

    assert!(result.is_err(), "the injected stop must fail");
    assert_eq!(fixture.pending(), 1);
    assert_eq!(fixture.bytes(), b"first-bytes");
    fixture.store.recover().expect("recover");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(rank(&fixture, &second.id), 1);
    assert_eq!(rank(&fixture, &first.id), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn profile_apply_plan_reports_changes_without_writing() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    move_to(&mut fixture, &second.id, 2);

    let plan = fixture
        .store
        .plan_profile_apply(&fixture.game, Some("default"))
        .expect("plan");

    assert!(plan.active);
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].to.as_deref(), Some(first.id.as_str()));
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
}
