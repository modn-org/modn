//! The refusals of a profile switch, in their documented order.

use super::plan_tests::{Fixture, SHARED};
use crate::error::{code_of, ErrorCode};
use std::fs;

#[test]
fn profile_switch_pending_transaction_refuses() {
    let fixture = Fixture::new();
    fixture.profile("target", false);
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO pending_transactions
             (id,action,status,game_path,package_id) VALUES ('t','install','pending',?1,'')",
            [fixture.game_text()],
        )
        .expect("pending");

    let error = fixture.plan("target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::PendingTransaction));
    assert!(format!("{error:#}").contains("PENDING_TRANSACTION"));
}

#[test]
fn profile_switch_unknown_profile_refuses() {
    let fixture = Fixture::new();

    let error = fixture.plan("missing").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileNotFound));
    assert!(format!("{error:#}").contains("PROFILE_NOT_FOUND"));
}

#[test]
fn profile_switch_active_target_refuses() {
    let fixture = Fixture::new();

    let error = fixture.plan("default").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileActive));
    assert!(format!("{error:#}").contains("PROFILE_ACTIVE"));
}

#[test]
fn profile_switch_changed_file_refuses() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fs::write(fixture.game.join(SHARED), b"user-edit").expect("edit");

    let error = fixture.plan("target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    let text = format!("{error:#}");
    assert!(text.contains("PROFILE_DRIFT"));
    assert!(text.contains("modn doctor"));
}

#[test]
fn profile_switch_missing_file_refuses() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fs::remove_file(fixture.game.join(SHARED)).expect("remove");

    let error = fixture.plan("target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    assert!(format!("{error:#}").contains("modn doctor"));
}

#[test]
fn profile_switch_missing_archives_lists_every_package() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/second.plugin", b"s")],
    );
    fixture.package("third", "target", 1, true, &[("addons/third.plugin", b"t")]);
    fixture.remove_cache();

    let error = fixture.plan("target").expect_err("refuse");
    let text = format!("{error:#}");
    let lines: Vec<&str> = text.lines().collect();

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert_eq!(lines.len(), 2);
    assert!(lines
        .iter()
        .all(|line| line.contains("PROFILE_ARCHIVE_MISSING")));
    assert!(text.contains("second"));
    assert!(text.contains("third"));
    assert!(text.contains("modn cache show"));
}

#[test]
fn profile_switch_archive_entry_mismatch_refuses() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package("second", "target", 0, true, &[(SHARED, b"second-bytes")]);
    fixture.tamper_claim("second", SHARED);

    let error = fixture.plan("target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    assert!(format!("{error:#}").contains("second"));
}

#[test]
fn profile_switch_unmanaged_collision_refuses() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package(
        "second",
        "target",
        0,
        true,
        &[("addons/unmanaged.plugin", b"s")],
    );
    fs::write(fixture.game.join("addons/unmanaged.plugin"), b"user-bytes").expect("unmanaged");

    let error = fixture.plan("target").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::Conflict));
    assert!(format!("{error:#}").contains("addons/unmanaged.plugin"));
}

#[test]
fn profile_switch_refusal_order_pending_before_profile() {
    let fixture = Fixture::new();
    fixture
        .store
        .connection
        .execute(
            "INSERT INTO pending_transactions
             (id,action,status,game_path,package_id) VALUES ('t','install','pending',?1,'')",
            [fixture.game_text()],
        )
        .expect("pending");

    let error = fixture.plan("missing").expect_err("refuse");

    assert_eq!(code_of(&error), Some(ErrorCode::PendingTransaction));
}
