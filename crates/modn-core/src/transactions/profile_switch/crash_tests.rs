//! Crash recovery of a profile switch at four points.
//!
//! Each test stops the switch without rollback, then runs the normal
//! `recover` entry point and checks the game tree, the active profile and the
//! manager rows. The fixture writes the rows an install would commit.

use super::execute::SwitchFault;
use super::plan_tests::Fixture;
use super::ProfileSwitchOptions;
use crate::error::{code_of, ErrorCode};
use std::fs;
use std::path::Path;

fn crash(fixture: &mut Fixture, target: &str, fault: SwitchFault) {
    let error = super::execute::switch_with_fault(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::default(),
        fault,
    )
    .expect_err("the injected crash must stop the switch");
    assert!(format!("{error:#}").contains("injected"));
}

fn active(fixture: &Fixture) -> String {
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

fn pending(fixture: &Fixture) -> i64 {
    fixture
        .store
        .connection
        .query_row("SELECT COUNT(*) FROM pending_transactions", [], |row| {
            row.get(0)
        })
        .expect("pending count")
}

fn history_len(fixture: &Fixture) -> usize {
    fixture
        .store
        .history(Some("test-game"))
        .expect("history")
        .len()
}

fn bytes(fixture: &Fixture, path: &str) -> Vec<u8> {
    fs::read(fixture.game.join(path)).expect("bytes")
}

/// Every `.modn-txn-*` witness below `root`.
fn witnesses(root: &Path) -> Vec<String> {
    fn walk(base: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(base).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".modn-txn-"))
            {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn staging_dirs(fixture: &Fixture) -> Vec<String> {
    fs::read_dir(&fixture.store.state_root)
        .expect("state root")
        .filter_map(|entry| {
            let entry = entry.expect("entry");
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("txn-")
                .then(|| entry.path().to_string_lossy().into_owned())
        })
        .collect()
}

/// A target profile that writes one new file and replaces one shared file.
fn replace_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[
            ("addons/shared.plugin", b"first"),
            ("addons/c.plugin", b"first-c"),
        ],
    );
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[
            ("addons/shared.plugin", b"second"),
            ("addons/b.plugin", b"second-b"),
        ],
    );
    fixture
}

#[test]
fn crash_after_journal_recovery_keeps_the_old_profile_and_tree() {
    let mut fixture = replace_fixture();
    let tree = fixture.tree();

    crash(&mut fixture, "target", SwitchFault::AfterJournal);

    assert_eq!(pending(&fixture), 1);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree, "the journal moved no game file");
    assert_eq!(history_len(&fixture), 0);

    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    assert_eq!(history_len(&fixture), 0);
    assert!(witnesses(&fixture.game).is_empty());
    assert!(staging_dirs(&fixture).is_empty());
}

#[test]
fn crash_after_some_quarantines_restores_every_removed_file() {
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
    let tree = fixture.tree();

    crash(&mut fixture, "target", SwitchFault::AfterQuarantines(1));

    assert_eq!(pending(&fixture), 1);
    assert_eq!(active(&fixture), "default");
    assert_eq!(history_len(&fixture), 0);
    assert_eq!(bytes(&fixture, "addons/b.plugin"), b"first-b");

    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    assert_eq!(bytes(&fixture, "addons/a.plugin"), b"first-a");
    assert!(witnesses(&fixture.game).is_empty());
}

#[test]
fn crash_after_all_moves_before_commit_restores_the_old_bytes() {
    let mut fixture = replace_fixture();
    let tree = fixture.tree();

    crash(&mut fixture, "target", SwitchFault::AfterMoves);

    assert_eq!(pending(&fixture), 1);
    assert_eq!(active(&fixture), "default");
    assert_eq!(history_len(&fixture), 0);
    assert_eq!(bytes(&fixture, "addons/shared.plugin"), b"second");
    assert_eq!(bytes(&fixture, "addons/b.plugin"), b"second-b");

    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    assert!(witnesses(&fixture.game).is_empty());
    assert!(staging_dirs(&fixture).is_empty());
}

#[test]
fn crash_after_commit_before_cleanup_keeps_the_new_profile() {
    let mut fixture = replace_fixture();

    crash(&mut fixture, "target", SwitchFault::AfterCommit);

    assert_eq!(pending(&fixture), 1, "cleanup waits for recovery");
    assert_eq!(active(&fixture), "target");
    assert_eq!(history_len(&fixture), 1);
    assert!(
        !witnesses(&fixture.game).is_empty(),
        "the `.old` witness remains before cleanup"
    );

    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "target");
    assert_eq!(history_len(&fixture), 1);
    assert!(witnesses(&fixture.game).is_empty());
    assert!(staging_dirs(&fixture).is_empty());
    assert_eq!(bytes(&fixture, "addons/shared.plugin"), b"second");
    assert_eq!(bytes(&fixture, "addons/b.plugin"), b"second-b");
    assert!(!fixture.game.join("addons/c.plugin").exists());
}

#[test]
fn crash_recovery_keeps_a_user_edited_file_and_reports_it() {
    let mut fixture = replace_fixture();
    let tree = fixture.tree();
    crash(&mut fixture, "target", SwitchFault::AfterMoves);
    fs::write(fixture.game.join("addons/shared.plugin"), b"user-edit").expect("edit");

    let error = fixture.store.recover().expect_err("report the edit");

    assert_eq!(code_of(&error), Some(ErrorCode::Conflict));
    assert_eq!(bytes(&fixture, "addons/shared.plugin"), b"user-edit");
    assert_eq!(pending(&fixture), 1, "the journal waits for a decision");

    fs::write(fixture.game.join("addons/shared.plugin"), b"second")
        .expect("put the switch bytes back");
    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
}

#[test]
fn crash_after_commit_with_witness_recovery_is_idempotent() {
    let mut fixture = replace_fixture();
    crash(&mut fixture, "target", SwitchFault::AfterCommit);

    fixture.store.recover().expect("first recover");
    fixture.store.recover().expect("second recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "target");
    assert_eq!(history_len(&fixture), 1);
}

#[test]
fn crash_after_journal_keeps_the_journal_evidence_until_recovery() {
    let mut fixture = replace_fixture();
    crash(&mut fixture, "target", SwitchFault::AfterJournal);

    let (action, package): (String, String) = fixture
        .store
        .connection
        .query_row(
            "SELECT action, package_id FROM pending_transactions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("journal");
    let recorded: (String, String) = fixture
        .store
        .connection
        .query_row(
            "SELECT from_profile, to_profile FROM pending_profile_switches",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("switch journal row");

    assert_eq!(action, "profile-switch");
    assert_eq!(package, "");
    assert_eq!(recorded, ("default".to_owned(), "target".to_owned()));
}
