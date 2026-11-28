//! Executing, committing and cancelling a profile switch.
//!
//! Every test uses a disposable game tree and manager state. The fixture
//! writes the rows an install would commit, so a switch can be checked
//! without running an archive install.

use super::execute::{self, SwitchFault};
use super::plan_tests::Fixture;
use super::{profile_switch, ProfileSwitchOptions, ProfileSwitchOutcome};
use crate::error::{code_of, ErrorCode};
use crate::progress::{CancellationToken, OperationControl};
use std::fs;

pub(super) fn switch(fixture: &mut Fixture, target: &str) -> anyhow::Result<ProfileSwitchOutcome> {
    profile_switch(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::default(),
    )
}

fn switch_fault(
    fixture: &mut Fixture,
    target: &str,
    fault: SwitchFault,
) -> anyhow::Result<ProfileSwitchOutcome> {
    execute::switch_with_fault(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::default(),
        fault,
    )
}

fn switch_cancelled_after_moves(
    fixture: &mut Fixture,
    target: &str,
) -> anyhow::Result<ProfileSwitchOutcome> {
    let token = CancellationToken::new();
    let control = OperationControl::new(token, |_| {});
    execute::switch_with_fault(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::new(control),
        SwitchFault::CancelAfterMoves,
    )
}

fn switch_with_control(
    fixture: &mut Fixture,
    target: &str,
    control: OperationControl,
) -> anyhow::Result<ProfileSwitchOutcome> {
    profile_switch(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::new(control),
    )
}

pub(super) fn active(fixture: &Fixture) -> String {
    fixture
        .store
        .connection
        .query_row(
            "SELECT name FROM profiles WHERE game_path=?1 AND active=1",
            [fixture.game_text()],
            |row| row.get(0),
        )
        .expect("active profile")
}

pub(super) fn pending(fixture: &Fixture) -> i64 {
    fixture
        .store
        .connection
        .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
            row.get(0)
        })
        .expect("pending count")
}

/// The winner rows as `(path, package, profile)` in ownership key order.
pub(super) fn winners(fixture: &Fixture) -> Vec<(String, String, String)> {
    let mut statement = fixture
        .store
        .connection
        .prepare(
            "SELECT relative_path, package_id, profile_id FROM managed_files
             WHERE game_path=?1 ORDER BY root, path_key",
        )
        .expect("winners");
    statement
        .query_map([fixture.game_text()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("winners")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("winners")
}

pub(super) fn switch_rows(fixture: &Fixture) -> Vec<(String, String)> {
    let mut statement = fixture
        .store
        .connection
        .prepare(
            "SELECT from_profile, to_profile FROM history_profile_switches
             ORDER BY operation_id",
        )
        .expect("switch rows");
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("switch rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("switch rows")
}

pub(super) fn bytes(fixture: &Fixture, path: &str) -> Vec<u8> {
    fs::read(fixture.game.join(path)).expect("bytes")
}

#[test]
fn profile_switch_round_trip_restores_the_original_tree() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[
            ("addons/a.plugin", b"first-a"),
            ("addons/b.plugin", b"first-b"),
        ],
    );
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[
            ("addons/a.plugin", b"second-a"),
            ("addons/c.plugin", b"second-c"),
        ],
    );
    let before = fixture.tree();

    let outcome = switch(&mut fixture, "target").expect("switch to target");
    assert_eq!(outcome.written, 2, "a is replaced and c is written");
    assert_eq!(outcome.removed, 1, "b leaves the active profile");
    assert_eq!(bytes(&fixture, "addons/a.plugin"), b"second-a");
    assert_eq!(bytes(&fixture, "addons/c.plugin"), b"second-c");
    assert!(!fixture.game.join("addons/b.plugin").exists());

    switch(&mut fixture, "default").expect("switch back to default");
    assert_eq!(fixture.tree(), before, "the original tree is back");
}

#[test]
fn profile_switch_replaces_a_shared_path_with_the_target_winner() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/shared.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/shared.plugin", b"second")],
    );

    let outcome = switch(&mut fixture, "target").expect("switch");

    assert_eq!(outcome.plan.write.len(), 1);
    assert_eq!(outcome.written, 1);
    assert_eq!(bytes(&fixture, "addons/shared.plugin"), b"second");
    assert_eq!(
        winners(&fixture),
        [(
            "addons/shared.plugin".to_owned(),
            "second".to_owned(),
            "profile-target".to_owned()
        )]
    );
}

#[test]
fn profile_switch_writes_entering_and_removes_leaving_paths() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );

    let outcome = switch(&mut fixture, "target").expect("switch");

    assert_eq!(outcome.written, 1);
    assert_eq!(outcome.removed, 1);
    assert_eq!(fixture.tree().len(), 1);
    assert_eq!(bytes(&fixture, "addons/second.plugin"), b"second");
    assert_eq!(outcome.plan.entering, ["second"]);
    assert_eq!(outcome.plan.leaving, ["first"]);
}

#[test]
fn profile_switch_keeps_unchanged_paths_and_moves_their_row() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("shared", "default", 0, &[("addons/shared.plugin", b"same")]);
    fixture.package(
        "copy",
        "target",
        0,
        true,
        &[("addons/shared.plugin", b"same")],
    );
    let before = fixture.tree();

    let outcome = switch(&mut fixture, "target").expect("switch");

    assert_eq!(outcome.written, 0);
    assert_eq!(outcome.unchanged, 1);
    assert_eq!(fixture.tree(), before);
    assert_eq!(
        winners(&fixture),
        [(
            "addons/shared.plugin".to_owned(),
            "copy".to_owned(),
            "profile-target".to_owned()
        )]
    );
}

#[test]
fn profile_switch_missing_archive_refuses_without_change() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );
    fixture.remove_cache();
    let tree = fixture.tree();
    let rows = fixture.state_rows();
    let before = winners(&fixture);

    let error = switch(&mut fixture, "target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert!(format!("{error:#}").contains("modn cache show"));
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.state_rows(), rows);
    assert_eq!(winners(&fixture), before);
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn profile_switch_drift_refuses_without_change() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );
    fs::write(fixture.game.join("addons/first.plugin"), b"user-edit").expect("drift");
    let rows = fixture.state_rows();
    let before = winners(&fixture);

    let error = switch(&mut fixture, "target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    assert!(format!("{error:#}").contains("modn doctor"));
    assert_eq!(bytes(&fixture, "addons/first.plugin"), b"user-edit");
    assert_eq!(fixture.state_rows(), rows);
    assert_eq!(winners(&fixture), before);
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn profile_switch_cancelled_before_start_changes_nothing() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );
    let tree = fixture.tree();
    let rows = fixture.state_rows();
    let token = CancellationToken::new();
    token.cancel();
    let control = OperationControl::new(token, |_| {});

    let error = switch_with_control(&mut fixture, "target", control).expect_err("cancelled");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.state_rows(), rows);
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn profile_switch_cancelled_after_moves_rolls_back() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[
            ("addons/a.plugin", b"first-a"),
            ("addons/c.plugin", b"first-c"),
        ],
    );
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[
            ("addons/a.plugin", b"second-a"),
            ("addons/b.plugin", b"second-b"),
        ],
    );
    let tree = fixture.tree();
    let rows = fixture.state_rows();

    let error = switch_cancelled_after_moves(&mut fixture, "target").expect_err("cancelled");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert_eq!(fixture.tree(), tree, "every game byte is back");
    assert_eq!(fixture.state_rows(), rows);
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn profile_switch_cancel_during_placement_rolls_back() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );
    let tree = fixture.tree();
    let token = CancellationToken::new();
    let cancelling = token.clone();
    let control = OperationControl::new(token, move |event| {
        if matches!(event, crate::progress::ProgressEvent::Stage { .. }) {
            cancelling.cancel();
        }
    });

    let error = switch_with_control(&mut fixture, "target", control).expect_err("cancelled");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert_eq!(fixture.tree(), tree);
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn profile_switch_fault_before_commit_keeps_the_old_profile() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/shared.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/shared.plugin", b"second")],
    );
    let tree = fixture.tree();
    let rows = winners(&fixture);

    let error = switch_fault(&mut fixture, "target", SwitchFault::AfterMoves).expect_err("stopped");

    assert!(format!("{error:#}").contains("injected"));
    assert_eq!(pending(&fixture), 1, "the journal waits for recovery");
    assert_eq!(active(&fixture), "default");
    assert_eq!(winners(&fixture), rows);
    assert!(
        fixture
            .store
            .history(Some("test-game"))
            .expect("history")
            .is_empty(),
        "no history committed with the stopped switch"
    );

    fixture.store.recover().expect("recover");
    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    assert!(
        fixture
            .store
            .history(Some("test-game"))
            .expect("history")
            .is_empty(),
        "recovery leaves no history record"
    );
}

#[test]
fn profile_switch_pending_journal_blocks_another_switch() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"second")],
    );
    let tree = fixture.tree();
    switch_fault(&mut fixture, "target", SwitchFault::AfterJournal).expect_err("stopped");

    let error = switch(&mut fixture, "target").expect_err("blocked");

    assert_eq!(code_of(&error), Some(ErrorCode::PendingTransaction));
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    fixture.store.recover().expect("recover");
    switch(&mut fixture, "target").expect("switch after recovery");
    assert_eq!(active(&fixture), "target");
}

#[test]
fn profile_switch_active_target_refuses() {
    let mut fixture = Fixture::new();
    fixture.install("first", "default", 0, &[("addons/first.plugin", b"first")]);
    let tree = fixture.tree();

    let error = switch(&mut fixture, "default").expect_err("already active");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileActive));
    assert_eq!(fixture.tree(), tree);
    assert_eq!(active(&fixture), "default");
}
