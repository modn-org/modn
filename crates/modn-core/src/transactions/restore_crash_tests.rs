//! Recovery of an interrupted uninstall that restores a claimant's bytes.

use super::journal::{restore_path, temp_path, JournalFile};
use super::restore::{self, PendingRestore, RestoreCandidate, StagedRestore};
use super::restore_tests::{Fixture, ENTRY};
use super::ManagedPackage;
use crate::state::RootKind;
use std::collections::BTreeMap;
use std::fs;

#[test]
fn restore_crash_before_witness_rolls_back() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let file = journal_file(&second);
    fixture
        .store
        .insert_journal(
            "txn-restore-before-witness",
            "uninstall",
            &fixture.game,
            &second.id,
            None,
            std::slice::from_ref(&file),
            &[],
        )
        .expect("journal");
    record_manual_restore(&mut fixture, "txn-restore-before-witness", &first);

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.installed(), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_crash_after_witness_before_placement_rolls_back() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = "txn-restore-after-witness";
    let file = journal_file(&second);
    fixture
        .store
        .insert_journal(
            id,
            "uninstall",
            &fixture.game,
            &second.id,
            None,
            std::slice::from_ref(&file),
            &[],
        )
        .expect("journal");
    record_manual_restore(&mut fixture, id, &first);
    let temp = temp_path(&fixture.path(), id, 0).expect("witness");
    fs::hard_link(fixture.path(), &temp).expect("backup");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert!(!temp.exists());
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_crash_before_commit_rolls_back_to_winner() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = fixture.crash_uninstall(&second.id);
    assert_eq!(fixture.bytes(), b"first-bytes");
    let restore = restore_path(&fixture.path(), &id, 0).expect("restore witness");
    assert!(restore.is_file());

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert!(!restore.exists());
    assert!(!temp_path(&fixture.path(), &id, 0).expect("temp").exists());
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.installed(), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_crash_after_commit_keeps_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = fixture.crash_uninstall(&second.id);
    fixture
        .store
        .commit_uninstall(&id, &second.id)
        .expect("commit");

    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert_eq!(fixture.installed(), 1);
    assert!(!restore_path(&fixture.path(), &id, 0)
        .expect("restore")
        .exists());
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_pending_rows_name_the_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let id = "txn-restore-rows";
    let file = journal_file(&second);
    fixture
        .store
        .insert_journal(
            id,
            "uninstall",
            &fixture.game,
            &second.id,
            None,
            std::slice::from_ref(&file),
            &[],
        )
        .expect("journal");
    record_manual_restore(&mut fixture, id, &first);
    let rows = restore::pending_restores(&fixture.store, id).expect("restore rows");
    assert_eq!(
        rows.get(&0),
        Some(&PendingRestore {
            package_id: first.id.clone(),
            archive_entry: ENTRY.into(),
            blake3: blake3::hash(b"first-bytes").to_hex().to_string(),
            bytes: 11,
        })
    );
}

fn journal_file(package: &ManagedPackage) -> JournalFile {
    let file = package
        .files
        .iter()
        .find(|file| file.destination == super::restore_tests::SHARED)
        .expect("shared file");
    JournalFile {
        ordinal: 0,
        root: RootKind::from_label(&file.root).unwrap_or(RootKind::Game),
        destination: file.destination.clone(),
        archive_entry: ENTRY.into(),
        blake3: file.blake3.clone(),
        bytes: file.bytes,
    }
}

fn record_manual_restore(fixture: &mut Fixture, id: &str, claimant: &ManagedPackage) {
    let file = claimant
        .files
        .iter()
        .find(|file| file.destination == super::restore_tests::SHARED)
        .expect("claimant file");
    let candidate = RestoreCandidate {
        package_id: claimant.id.clone(),
        destination: super::restore_tests::SHARED.into(),
        archive_entry: ENTRY.into(),
        blake3: file.blake3.clone(),
        bytes: file.bytes,
    };
    let staged = fixture.store.state_root.join(format!("{id}-restore.bin"));
    fs::write(&staged, b"first-bytes").expect("staged bytes");
    let mut restores = BTreeMap::new();
    restores.insert(0, StagedRestore { candidate, staged });
    restore::record_restores(&mut fixture.store, id, &restores).expect("restore rows");
    fs::remove_file(restores.get(&0).expect("restore").staged.clone()).expect("drop staged bytes");
}
