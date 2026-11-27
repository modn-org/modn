//! Committed profile switch state: history, winners and counts.
//!
//! Every test uses a disposable game tree and manager state.

use super::execute_tests::{active, bytes, pending, switch, switch_rows, winners};
use super::plan_profile_switch;
use super::plan_tests::Fixture;
use crate::error::{code_of, ErrorCode};
use crate::history::HistoryKind;
use crate::transactions::undo::{conflict, ensure_undo_target};
use rusqlite::params;
use std::fs;

#[test]
fn profile_switch_commits_history_and_active_flag_with_the_winners() {
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

    assert_eq!(active(&fixture), "target");
    assert_eq!(pending(&fixture), 0);
    assert_eq!(
        winners(&fixture),
        [(
            "addons/second.plugin".to_owned(),
            "second".to_owned(),
            "profile-target".to_owned()
        )]
    );
    assert_eq!(
        switch_rows(&fixture),
        [("default".to_owned(), "target".to_owned())]
    );
    let history = fixture.store.history(Some("test-game")).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].kind, HistoryKind::ProfileSwitch);
    assert_eq!(history[0].id, outcome.transaction_id);
    assert!(
        history[0].packages.iter().any(|package| {
            package.files.iter().any(|file| {
                file.destination == "addons/second.plugin"
                    && file.digest_before.is_none()
                    && file.digest_after.is_some()
            })
        }),
        "the written file is recorded with its after digest"
    );
    assert_eq!(
        fixture.store.committed_operation_id(),
        Some(outcome.transaction_id.as_str())
    );
}

#[test]
fn profile_switch_outcome_matches_the_plan_and_reports_counts() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[("addons/a.plugin", b"a"), ("addons/b.plugin", b"b")],
    );
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/a.plugin", b"second-a")],
    );
    let plan = plan_profile_switch(&fixture.store, &fixture.game, "target").expect("plan");

    let outcome = switch(&mut fixture, "target").expect("switch");

    assert_eq!(outcome.plan, plan);
    assert_eq!(outcome.written, plan.write.len());
    assert_eq!(outcome.removed, plan.remove.len());
    assert_eq!(outcome.unchanged, plan.unchanged);
    assert_eq!(outcome.plan.bytes_to_write, 8);
    assert!(outcome.transaction_id.starts_with("txn-"));
}

#[test]
fn profile_switch_history_cannot_be_undone() {
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
    switch(&mut fixture, "target").expect("switch");
    let operations = fixture.store.history(Some("test-game")).expect("history");

    let error = ensure_undo_target(&operations[0]).expect_err("refuse undo");
    let found = conflict(&operations, &operations[0]).expect("conflict");

    assert_eq!(code_of(&error), Some(ErrorCode::UndoConflict));
    assert!(format!("{error:#}").contains("reversing a switch is a new switch"));
    assert_eq!(found.kind, HistoryKind::ProfileSwitch);
    assert_eq!(found.operation_id, operations[0].id);
    assert!(found.reason.contains("new switch"));
}

#[test]
fn profile_switch_records_before_and_after_digests_of_a_replaced_file() {
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

    switch(&mut fixture, "target").expect("switch");
    let operations = fixture.store.history(Some("test-game")).expect("history");
    let removed = operations[0]
        .packages
        .iter()
        .find(|package| package.package_id_before.as_deref() == Some("first"))
        .expect("leaving package");
    let written = operations[0]
        .packages
        .iter()
        .find(|package| package.package_id_after.as_deref() == Some("second"))
        .expect("entering package");

    assert_eq!(
        removed.files[0].digest_before.as_deref(),
        Some(blake3::hash(b"first").to_hex().as_str())
    );
    assert_eq!(removed.files[0].digest_after, None);
    assert_eq!(
        written.files[0].digest_before.as_deref(),
        Some(blake3::hash(b"first").to_hex().as_str())
    );
    assert_eq!(
        written.files[0].digest_after.as_deref(),
        Some(blake3::hash(b"second").to_hex().as_str())
    );
}
#[test]
fn profile_switch_empty_target_removes_every_managed_path() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[("addons/a.plugin", b"a"), ("addons/b.plugin", b"b")],
    );

    let outcome = switch(&mut fixture, "target").expect("switch");

    assert_eq!(outcome.removed, 2);
    assert_eq!(outcome.written, 0);
    assert!(fixture.tree().is_empty());
    assert!(winners(&fixture).is_empty());
    assert_eq!(active(&fixture), "target");
    assert_eq!(
        switch_rows(&fixture),
        [("default".to_owned(), "target".to_owned())]
    );
}

#[test]
fn profile_switch_leaves_unmanaged_files_untouched() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[("addons/managed.plugin", b"managed")],
    );
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/other.plugin", b"other")],
    );
    fs::write(fixture.game.join("addons/user.cfg"), b"user").expect("unmanaged");

    switch(&mut fixture, "target").expect("switch");

    assert_eq!(bytes(&fixture, "addons/user.cfg"), b"user");
}

#[test]
fn profile_switch_keeps_managed_directories() {
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
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO managed_directories
             (game_path,root,path_key,relative_path,package_id,identity)
             VALUES (?1,'game',?2,'addons/Exclusive','first','identity')",
            params![fixture.game_text(), "addons/exclusive"],
        )
        .expect("managed directory");

    switch(&mut fixture, "target").expect("switch");

    let directories: i64 = fixture
        .store
        .connection
        .query_row("SELECT COUNT(*) FROM managed_directories", [], |row| {
            row.get(0)
        })
        .expect("directories");
    assert_eq!(directories, 1);
}
