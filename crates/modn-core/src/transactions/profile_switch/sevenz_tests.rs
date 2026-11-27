//! Seven-zip archives in a profile switch.
//!
//! A switch stages every write into one transaction directory and places each
//! staged member by its ordinal. These tests seed the cache with real 7z
//! archives so several 7z writes share that directory.

use super::execute::{self, SwitchFault};
use super::plan_tests::Fixture;
use super::ProfileSwitchOptions;
use crate::error::{code_of, ErrorCode};
use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
use std::fs;
use std::io::Cursor;

fn switch(fixture: &mut Fixture, target: &str) -> anyhow::Result<super::ProfileSwitchOutcome> {
    super::profile_switch(
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
) -> anyhow::Result<super::ProfileSwitchOutcome> {
    execute::switch_with_fault(
        &mut fixture.store,
        &fixture.game,
        target,
        ProfileSwitchOptions::default(),
        fault,
    )
}

/// Replace a fixture package's ZIP cache entry with a 7z archive of the same
/// members and pin the package to the new archive.
fn sevenz_cache(fixture: &Fixture, name: &str, files: &[(&str, &[u8])]) {
    let cache = fixture.store.state_root.join("cache");
    let target = cache.join(format!("test-game#{name}#1.0#0000000.7z"));
    let mut writer = ArchiveWriter::create(&target).expect("7z writer");
    for (entry, bytes) in files {
        writer
            .push_archive_entry(ArchiveEntry::new_file(*entry), Some(Cursor::new(*bytes)))
            .expect("7z member");
    }
    writer.finish().expect("complete 7z");
    let pin = super::super::journal::digest(&target).expect("7z pin");
    fixture
        .store
        .connection
        .execute(
            "UPDATE managed_packages SET archive_blake3=?1 WHERE id=?2",
            rusqlite::params![format!("blake3:{pin}"), name],
        )
        .expect("pin 7z package");
    fs::remove_file(cache.join(format!("test-game#{name}#1.0#0000000.zip")))
        .expect("replace zip cache entry");
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

fn bytes(fixture: &Fixture, path: &str) -> Vec<u8> {
    fs::read(fixture.game.join(path)).expect("bytes")
}

/// Every `txn-` directory below the manager state root.
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

fn two_members() -> &'static [(&'static str, &'static [u8])] {
    &[("addons/one.plugin", b"one"), ("addons/two.plugin", b"two")]
}

#[test]
fn sevenz_switch_stages_two_members_of_one_archive() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.package("pack", "target", 0, true, two_members());
    sevenz_cache(&fixture, "pack", two_members());

    let outcome = switch(&mut fixture, "target").expect("switch from one 7z archive");

    assert_eq!(outcome.written, 2);
    assert_eq!(outcome.removed, 0);
    assert_eq!(bytes(&fixture, "addons/one.plugin"), b"one");
    assert_eq!(bytes(&fixture, "addons/two.plugin"), b"two");
    assert_eq!(active(&fixture), "target");
    assert_eq!(pending(&fixture), 0);
    assert!(staging_dirs(&fixture).is_empty());
}

#[test]
fn sevenz_switch_stages_members_of_two_archives() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    let alpha: &[(&str, &[u8])] = &[("addons/alpha.plugin", b"alpha")];
    let beta: &[(&str, &[u8])] = &[("addons/beta.plugin", b"beta")];
    fixture.package("alpha", "target", 1, true, alpha);
    fixture.package("beta", "target", 0, true, beta);
    sevenz_cache(&fixture, "alpha", alpha);
    sevenz_cache(&fixture, "beta", beta);

    let outcome = switch(&mut fixture, "target").expect("switch from two 7z archives");

    assert_eq!(outcome.written, 2);
    assert_eq!(bytes(&fixture, "addons/alpha.plugin"), b"alpha");
    assert_eq!(bytes(&fixture, "addons/beta.plugin"), b"beta");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn sevenz_switch_writes_a_7z_member_after_a_zip_member() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    let zip_files: &[(&str, &[u8])] = &[("addons/a-zip.plugin", b"zip-bytes")];
    let sevenz_files: &[(&str, &[u8])] = &[("addons/b-sevenz.plugin", b"sevenz-bytes")];
    fixture.package("zip-pack", "target", 1, true, zip_files);
    fixture.package("sevenz-pack", "target", 0, true, sevenz_files);
    sevenz_cache(&fixture, "sevenz-pack", sevenz_files);

    let outcome = switch(&mut fixture, "target").expect("switch after a zip write");

    assert_eq!(outcome.written, 2);
    assert_eq!(bytes(&fixture, "addons/a-zip.plugin"), b"zip-bytes");
    assert_eq!(bytes(&fixture, "addons/b-sevenz.plugin"), b"sevenz-bytes");
    assert_eq!(pending(&fixture), 0);
}

#[test]
fn sevenz_switch_cancel_after_moves_rolls_back_without_scratch() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.package("pack", "target", 0, true, two_members());
    sevenz_cache(&fixture, "pack", two_members());
    let tree = fixture.tree();

    let error =
        switch_fault(&mut fixture, "target", SwitchFault::CancelAfterMoves).expect_err("cancelled");

    assert_eq!(code_of(&error), Some(ErrorCode::Cancelled));
    assert_eq!(fixture.tree(), tree, "every game byte is back");
    assert_eq!(active(&fixture), "default");
    assert_eq!(pending(&fixture), 0);
    assert!(
        staging_dirs(&fixture).is_empty(),
        "no scratch directory stays"
    );
}

#[test]
fn sevenz_switch_crash_after_moves_recovers_the_old_profile() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.package("pack", "target", 0, true, two_members());
    sevenz_cache(&fixture, "pack", two_members());
    let tree = fixture.tree();

    let error = switch_fault(&mut fixture, "target", SwitchFault::AfterMoves).expect_err("stopped");
    assert!(format!("{error:#}").contains("injected"));
    assert_eq!(pending(&fixture), 1, "the journal waits for recovery");
    assert_eq!(active(&fixture), "default");
    assert_eq!(bytes(&fixture, "addons/one.plugin"), b"one");

    fixture.store.recover().expect("recover");

    assert_eq!(pending(&fixture), 0);
    assert_eq!(active(&fixture), "default");
    assert_eq!(fixture.tree(), tree);
    assert!(staging_dirs(&fixture).is_empty());
}
