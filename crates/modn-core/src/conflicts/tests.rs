use super::*;
use crate::error::{code_of, ErrorCode};
use rusqlite::params;
use std::path::Path;
use tempfile::tempdir;

const GAME: &str = "/game";

struct Fixture {
    _temp: tempfile::TempDir,
    store: Store,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempdir().expect("fixture");
        let store = Store::open(&temp.path().join("state.sqlite")).expect("state");
        Self { _temp: temp, store }
    }

    fn package(&self, id: &str) {
        self.store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,'hash')",
                params![id, GAME],
            )
            .expect("package");
    }

    fn profile(&self, id: &str, name: &str, active: bool) {
        self.store
            .connection
            .execute(
                "INSERT INTO profiles(id,game_path,name,active) VALUES (?1,?2,?3,?4)",
                params![id, GAME, name, active as i64],
            )
            .expect("profile");
    }

    fn member(&self, profile: &str, package: &str, priority: i64, enabled: bool) {
        self.store
            .connection
            .execute(
                "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
                 VALUES (?1,?2,?3,?4)",
                params![profile, package, priority, enabled as i64],
            )
            .expect("membership");
    }

    fn claim(&self, package: &str, root: &str, path: &str, digest: &str, bytes: i64) {
        self.store
            .connection
            .execute(
                "INSERT INTO file_claims
                 (package_id,game_path,root,path_key,relative_path,archive_entry,blake3,bytes)
                 VALUES (?1,?2,?3,?4,?4,?4,?5,?6)",
                params![package, GAME, root, path, digest, bytes],
            )
            .expect("claim");
    }

    fn winner(&self, profile: &str, root: &str, path: &str, package: &str) {
        self.store
            .connection
            .execute(
                "INSERT INTO managed_files
                 (game_path,root,path_key,relative_path,package_id,blake3,bytes,
                  profile_id,archive_entry)
                 VALUES (?1,?2,?3,?3,?4,'winner',1,?5,?3)",
                params![GAME, root, path, package, profile],
            )
            .expect("winner");
    }

    fn report(&self, profile: Option<&str>) -> ConflictReport {
        conflicts(&self.store, Path::new(GAME), profile).expect("conflict report")
    }
}

fn two_claiming_packages(fixture: &Fixture) {
    fixture.package("first");
    fixture.package("second");
    fixture.profile("default-id", "default", true);
    fixture.member("default-id", "first", 0, true);
    fixture.member("default-id", "second", 1, true);
}

#[test]
fn no_overlap_is_an_empty_report() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    fixture.claim("first", "game", "archive/one.archive", "one", 1);
    fixture.claim("second", "game", "archive/two.archive", "two", 2);

    let report = fixture.report(None);

    assert_eq!(report.profile, "default");
    assert!(report.active);
    assert!(report.conflicts.is_empty());
}

#[test]
fn two_overlapping_packages_put_the_later_install_at_rank_one() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    fixture.claim("first", "game", "archive/shared.archive", "first", 3);
    fixture.claim("second", "game", "archive/shared.archive", "second", 4);
    fixture.winner("default-id", "game", "archive/shared.archive", "second");

    let report = fixture.report(None);
    let conflict = &report.conflicts[0];

    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(conflict.root, "game");
    assert_eq!(conflict.path, "archive/shared.archive");
    assert_eq!(conflict.winner.as_deref(), Some("second"));
    assert!(conflict.consistent);
    assert_eq!(conflict.claims.len(), 2);
    assert_eq!(conflict.claims[0].package, "second");
    assert_eq!(conflict.claims[0].rank, 1);
    assert_eq!(conflict.claims[0].priority, 1);
    assert!(conflict.claims[0].winner);
    assert_eq!(conflict.claims[0].blake3, "second");
    assert_eq!(conflict.claims[0].bytes, 4);
    assert_eq!(conflict.claims[1].package, "first");
    assert_eq!(conflict.claims[1].rank, 2);
    assert_eq!(conflict.claims[1].priority, 0);
    assert!(!conflict.claims[1].winner);
}

#[test]
fn a_three_way_overlap_lists_every_claim_in_priority_order() {
    let fixture = Fixture::new();
    fixture.package("low");
    fixture.package("middle");
    fixture.package("high");
    fixture.profile("default-id", "default", true);
    fixture.member("default-id", "low", 0, true);
    fixture.member("default-id", "middle", 2, true);
    fixture.member("default-id", "high", 5, true);
    for package in ["low", "middle", "high"] {
        fixture.claim(package, "game", "archive/shared.archive", package, 10);
    }
    fixture.winner("default-id", "game", "archive/shared.archive", "high");

    let report = fixture.report(None);
    let claims = &report.conflicts[0].claims;

    assert_eq!(
        claims
            .iter()
            .map(|claim| claim.package.as_str())
            .collect::<Vec<_>>(),
        ["high", "middle", "low"]
    );
    assert_eq!(
        claims.iter().map(|claim| claim.rank).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        claims.iter().map(|claim| claim.winner).collect::<Vec<_>>(),
        [true, false, false]
    );
}

#[test]
fn a_disabled_package_is_not_a_claimant() {
    let fixture = Fixture::new();
    fixture.package("disabled");
    fixture.package("low");
    fixture.package("high");
    fixture.profile("default-id", "default", true);
    fixture.member("default-id", "disabled", 9, false);
    fixture.member("default-id", "low", 0, true);
    fixture.member("default-id", "high", 1, true);
    for package in ["disabled", "low", "high"] {
        fixture.claim(package, "game", "archive/shared.archive", package, 10);
    }
    fixture.winner("default-id", "game", "archive/shared.archive", "high");

    let report = fixture.report(None);

    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(
        report.conflicts[0]
            .claims
            .iter()
            .map(|claim| claim.package.as_str())
            .collect::<Vec<_>>(),
        ["high", "low"]
    );
}

#[test]
fn an_inactive_profile_has_no_stored_winner() {
    let fixture = Fixture::new();
    fixture.package("one");
    fixture.package("two");
    fixture.profile("spare-id", "spare", false);
    fixture.member("spare-id", "one", 0, true);
    fixture.member("spare-id", "two", 1, true);
    fixture.claim("one", "game", "archive/shared.archive", "one", 1);
    fixture.claim("two", "game", "archive/shared.archive", "two", 2);
    fixture.winner("spare-id", "game", "archive/shared.archive", "two");

    let report = fixture.report(Some("spare"));

    assert_eq!(report.profile, "spare");
    assert!(!report.active);
    assert!(report.conflicts[0].winner.is_none());
    assert!(report.conflicts[0].consistent);
    assert_eq!(report.conflicts[0].claims[0].package, "two");
}

#[test]
fn a_stored_winner_that_differs_is_inconsistent() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    fixture.claim("first", "game", "archive/shared.archive", "first", 3);
    fixture.claim("second", "game", "archive/shared.archive", "second", 4);
    fixture.winner("default-id", "game", "archive/shared.archive", "first");

    let report = fixture.report(None);
    let conflict = &report.conflicts[0];

    assert_eq!(conflict.winner.as_deref(), Some("first"));
    assert!(!conflict.consistent);
}

#[test]
fn a_missing_stored_winner_is_inconsistent_on_the_active_profile() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    fixture.claim("first", "game", "archive/shared.archive", "first", 3);
    fixture.claim("second", "game", "archive/shared.archive", "second", 4);

    let report = fixture.report(None);

    assert!(report.conflicts[0].winner.is_none());
    assert!(!report.conflicts[0].consistent);
}

#[test]
fn paths_are_sorted_by_root_and_ownership_key() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    for (root, path) in [
        ("game", "z.shared"),
        ("content", "m.shared"),
        ("game", "a.shared"),
    ] {
        fixture.claim("first", root, path, "first", 1);
        fixture.claim("second", root, path, "second", 2);
        fixture.winner("default-id", root, path, "second");
    }

    let report = fixture.report(None);

    assert_eq!(
        report
            .conflicts
            .iter()
            .map(|conflict| (conflict.root.as_str(), conflict.path.as_str()))
            .collect::<Vec<_>>(),
        [
            ("content", "m.shared"),
            ("game", "a.shared"),
            ("game", "z.shared"),
        ]
    );
}

#[test]
fn an_unknown_profile_fails_with_profile_not_found() {
    let fixture = Fixture::new();
    fixture.profile("default-id", "default", true);

    let error = conflicts(&fixture.store, Path::new(GAME), Some("missing"))
        .expect_err("unknown profile must fail");

    assert_eq!(code_of(&error), Some(ErrorCode::ProfileNotFound));
    assert!(format!("{error:#}").contains("missing"));
}

#[test]
fn without_a_name_the_active_profile_is_reported() {
    let fixture = Fixture::new();
    two_claiming_packages(&fixture);
    fixture.profile("spare-id", "spare", false);
    fixture.package("spare-package");
    fixture.member("spare-id", "spare-package", 0, true);
    fixture.claim("first", "game", "archive/active.archive", "first", 1);
    fixture.claim("second", "game", "archive/active.archive", "second", 2);
    fixture.winner("default-id", "game", "archive/active.archive", "second");

    let report = fixture.report(None);

    assert_eq!(report.profile, "default");
    assert!(report.active);
    assert_eq!(report.conflicts[0].path, "archive/active.archive");
}
