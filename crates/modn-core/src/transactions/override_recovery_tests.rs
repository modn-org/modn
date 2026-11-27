//! Recovery of an install that replaces a lower-priority managed file.
//!
//! Each test builds a real interrupted state and checks disk bytes, winner
//! rows, claim rows and surviving unmanaged files.

use super::install::Fault;
use super::journal::{backup_path, lock_store, temp_path, JournalFile};
use super::{ManagedPackage, NativeInjectorPolicy};
use crate::archive::inspect;
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const ONE: &str = "addons/shared-one.plugin";
const TWO: &str = "addons/shared-two.plugin";
const UNMANAGED: &str = "addons/user-notes.txt";
const OLD: &[u8] = b"first-bytes";
const NEW: &[u8] = b"second-bytes";

struct Fixture {
    _dir: TempDir,
    store: Store,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().expect("fixture");
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("bin")).expect("bin");
        fs::write(game.join("bin/TestGame.exe"), b"game-exe").expect("exe");
        fs::create_dir_all(game.join("addons")).expect("addons");
        fs::write(game.join(UNMANAGED), b"user-bytes").expect("unmanaged");
        let game = game.canonicalize().expect("canonical game");
        let store = Store::open(&dir.path().join("state/state.sqlite")).expect("state");
        Self {
            _dir: dir,
            store,
            game,
        }
    }

    fn archive(&self, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = self._dir.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(File::create(&path).expect("zip"));
        for (entry, bytes) in entries {
            zip.start_file(*entry, SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(bytes).expect("bytes");
        }
        zip.finish().expect("finish");
        path
    }

    fn plan(&self, archive: &Path, destinations: &[(&str, &str)]) -> InstallPlan {
        let checked = inspect(archive).expect("inspect");
        let operations = checked
            .files
            .into_iter()
            .map(|entry| {
                let destination = destinations
                    .iter()
                    .find(|(source, _)| *source == entry.entry)
                    .map(|(_, destination)| (*destination).to_owned())
                    .expect("planned destination");
                PlannedFile {
                    source: entry.entry,
                    destination,
                    bytes: entry.bytes,
                    kind: "file-copy".into(),
                    owned_directory: None,
                    root: "game".into(),
                }
            })
            .collect();
        InstallPlan {
            archive_blake3: inspect(archive).expect("re-inspect").blake3,
            game_installation: self.game.clone(),
            status: PlanStatus::Reviewable,
            metadata_only: true,
            operations,
            blockers: Vec::new(),
            skipped_generated_files: Vec::new(),
            content_base: None,
            content_root: None,
            overrides: Vec::new(),
        }
    }

    fn install(&mut self, archive: &Path, destinations: &[(&str, &str)]) -> ManagedPackage {
        let plan = self.plan(archive, destinations);
        let game = self.game.clone();
        self.store
            .install_operations_zip_with_permissions(&game, archive, &plan, &[])
            .expect("install")
    }

    /// Run the locked install with an injected crash after the first file.
    ///
    /// The journal, staging and witnesses stay as the crash leaves them.
    fn crash(&mut self, archive: &Path, destinations: &[(&str, &str)]) -> String {
        let plan = self.plan(archive, destinations);
        {
            let _guard = lock_store(&self.store).expect("transaction lock");
            assert!(self
                .store
                .install_locked(
                    &self.game,
                    archive,
                    &plan,
                    Some(Fault::CrashAfter(1)),
                    &[],
                    None,
                    NativeInjectorPolicy::Refuse,
                    None,
                )
                .is_err());
        }
        let pending = self.store.pending().expect("pending");
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    }

    /// Emulate the commit boundary of crash point 4 with the real commit code.
    fn commit(&mut self, id: &str) {
        let journal = self
            .store
            .pending()
            .expect("pending")
            .into_iter()
            .find(|journal| journal.id == id)
            .expect("journal");
        self.store
            .commit_install(
                id,
                &self.game,
                "archive",
                &journal.files,
                &[],
                None,
                None,
                None,
                None,
            )
            .expect("commit");
    }

    fn dest(&self, relative: &str) -> PathBuf {
        self.game.join(relative)
    }

    fn witness(&self, id: &str, relative: &str, ordinal: usize) -> (PathBuf, PathBuf) {
        let dest = self.dest(relative);
        (
            temp_path(&dest, id, ordinal).expect("temp witness"),
            backup_path(&dest, id, ordinal).expect("old witness"),
        )
    }

    fn count(&self, sql: &str) -> i64 {
        self.store
            .connection
            .query_row(sql, [], |row| row.get(0))
            .expect("count")
    }

    fn winner(&self, relative: &str) -> Option<(String, String)> {
        self.store
            .connection
            .query_row(
                "SELECT package_id, blake3 FROM managed_files WHERE relative_path=?1",
                [relative],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok()
    }

    fn assert_old_winner(&self, relative: &str, package: &ManagedPackage, claims: i64) {
        let expected = package
            .files
            .iter()
            .find(|file| file.destination == relative)
            .expect("owned file")
            .blake3
            .clone();
        assert_eq!(
            self.winner(relative),
            Some((package.id.clone(), expected)),
            "old owner stays the winner at {relative}"
        );
        assert_eq!(self.count("SELECT COUNT(*) FROM file_claims"), claims);
        assert_eq!(
            fs::read(self.game.join(UNMANAGED)).expect("unmanaged"),
            b"user-bytes"
        );
    }

    fn assert_old_owner(&self, relative: &str, package: &ManagedPackage, claims: i64) {
        assert_eq!(
            fs::read(self.dest(relative)).expect("bytes"),
            OLD,
            "old bytes at {relative}"
        );
        self.assert_old_winner(relative, package, claims);
        assert_eq!(self.count("SELECT COUNT(*) FROM managed_files"), claims);
        assert!(self.store.pending().expect("pending").is_empty());
    }

    fn assert_new_owner(&self, relative: &str, id: &str) {
        assert_eq!(fs::read(self.dest(relative)).expect("bytes"), NEW);
        let expected = blake3::hash(NEW).to_hex().to_string();
        assert_eq!(
            self.winner(relative),
            Some((id.to_owned(), expected)),
            "new owner is the winner at {relative}"
        );
        assert_eq!(
            self.count("SELECT COUNT(*) FROM managed_files"),
            1,
            "one winner row per path"
        );
        assert_eq!(
            self.count("SELECT COUNT(*) FROM file_claims"),
            2,
            "both claims survive the commit"
        );
        assert_eq!(
            fs::read(self.game.join(UNMANAGED)).expect("unmanaged"),
            b"user-bytes"
        );
        assert!(self.store.pending().expect("pending").is_empty());
    }

    fn assert_no_witnesses(&self, id: &str, relative: &str, ordinal: usize) {
        let (temp, backup) = self.witness(id, relative, ordinal);
        assert!(!temp.exists(), "temp witness removed: {}", temp.display());
        assert!(
            !backup.exists(),
            "old witness removed: {}",
            backup.display()
        );
    }
}

fn prepared_file(destination: &str, bytes: &[u8]) -> JournalFile {
    JournalFile {
        ordinal: 0,
        root: crate::state::RootKind::Game,
        destination: destination.into(),
        archive_entry: "pack/one.bin".into(),
        blake3: blake3::hash(bytes).to_hex().to_string(),
        bytes: bytes.len() as u64,
    }
}

#[test]
fn override_recovery_before_witness_leaves_state_unchanged() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let file = prepared_file(ONE, NEW);
    let id = "txn-before-witness";
    fixture
        .store
        .insert_journal(
            id,
            "install",
            &fixture.game,
            id,
            Some("archive"),
            std::slice::from_ref(&file),
            &[],
        )
        .expect("journal");
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    fixture.assert_old_owner(ONE, &first, 1);
}

#[test]
fn override_recovery_after_witness_before_placement_restores_old_bytes() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    fs::remove_file(fixture.dest(ONE)).expect("interrupted before placement");
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    fixture.assert_old_owner(ONE, &first, 1);
    fixture.assert_no_witnesses(&id, ONE, 0);
    assert_eq!(fixture.store.recover().expect("idle"), 0);

    // The same step interrupted before the old bytes are removed.
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    let (_temp, backup) = fixture.witness(&id, ONE, 0);
    fs::remove_file(fixture.dest(ONE)).expect("interrupted placement");
    fs::hard_link(&backup, fixture.dest(ONE)).expect("old bytes stay in place");
    assert_eq!(fixture.store.recover().expect("recover old in place"), 1);
    fixture.assert_old_owner(ONE, &first, 1);
    fixture.assert_no_witnesses(&id, ONE, 0);
}

#[test]
fn override_recovery_after_placement_before_commit_restores_old_owner() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    assert_eq!(fs::read(fixture.dest(ONE)).expect("new bytes"), NEW);
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    fixture.assert_old_owner(ONE, &first, 1);
    fixture.assert_no_witnesses(&id, ONE, 0);
}

#[test]
fn override_recovery_after_commit_keeps_new_file_and_state() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    fixture.commit(&id);
    let status: String = fixture
        .store
        .connection
        .query_row(
            "SELECT status FROM pending_transactions WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .expect("status");
    assert_eq!(status, "committed");
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    fixture.assert_new_owner(ONE, &id);
    fixture.assert_no_witnesses(&id, ONE, 0);
}

#[test]
fn override_recovery_user_edited_destination_is_refused() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    let dest = fixture.dest(ONE);
    fs::write(&dest, b"user-edit").expect("user edit");
    let (temp, backup) = fixture.witness(&id, ONE, 0);

    let error = fixture.store.recover().expect_err("refuse");
    let message = format!("{error:#}");
    assert!(message.contains("CONFLICT"), "{message}");
    assert!(message.contains("destination changed"), "{message}");
    assert_eq!(fs::read(&dest).expect("bytes"), b"user-edit");
    assert!(temp.exists() && backup.exists(), "witnesses preserved");
    assert_eq!(fixture.store.pending().expect("pending").len(), 1);
    fixture.assert_old_winner(ONE, &first, 1);
}

#[test]
fn override_recovery_multifile_restores_each_file_by_its_rule() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive(
        "first",
        &[("pack/one.bin", OLD), ("pack/two.bin", b"first-two")],
    );
    let first = fixture.install(
        &first_archive,
        &[("pack/one.bin", ONE), ("pack/two.bin", TWO)],
    );
    let second_archive = fixture.archive(
        "second",
        &[("pack/one.bin", NEW), ("pack/two.bin", b"second-two")],
    );
    let id = fixture.crash(
        &second_archive,
        &[("pack/one.bin", ONE), ("pack/two.bin", TWO)],
    );
    assert_eq!(fs::read(fixture.dest(ONE)).expect("placed"), NEW);
    assert_eq!(
        fs::read(fixture.dest(TWO)).expect("untouched"),
        b"first-two"
    );
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    assert_eq!(fs::read(fixture.dest(ONE)).expect("one"), OLD);
    assert_eq!(fs::read(fixture.dest(TWO)).expect("two"), b"first-two");
    for relative in [ONE, TWO] {
        let expected = first
            .files
            .iter()
            .find(|file| file.destination == relative)
            .expect("owned file")
            .blake3
            .clone();
        assert_eq!(
            fixture.winner(relative),
            Some((first.id.clone(), expected)),
            "{relative}"
        );
    }
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM managed_files"),
        2,
        "both files keep one winner row"
    );
    assert_eq!(
        fixture.count("SELECT COUNT(*) FROM file_claims"),
        2,
        "no shadowed claim was committed"
    );
    assert_eq!(
        fs::read(fixture.game.join(UNMANAGED)).expect("unmanaged"),
        b"user-bytes"
    );
    fixture.assert_no_witnesses(&id, ONE, 0);
    assert!(fixture.store.pending().expect("pending").is_empty());
}

#[test]
fn override_recovery_is_idempotent() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);

    assert_eq!(fixture.store.recover().expect("first recovery"), 1);
    assert_eq!(fixture.store.recover().expect("second recovery"), 0);
    fixture.assert_old_owner(ONE, &first, 1);
    fixture.assert_no_witnesses(&id, ONE, 0);
}

#[test]
fn override_recovery_missing_witness_is_refused() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    let (temp, backup) = fixture.witness(&id, ONE, 0);
    fs::remove_file(&backup).expect("lost witness");

    let error = fixture.store.recover().expect_err("refuse");
    let message = format!("{error:#}");
    assert!(message.contains("CONFLICT"), "{message}");
    assert!(message.contains("no override witness"), "{message}");
    assert_eq!(fs::read(fixture.dest(ONE)).expect("new bytes"), NEW);
    assert!(temp.exists(), "staged bytes preserved");
    assert_eq!(fixture.store.pending().expect("pending").len(), 1);
    fixture.assert_old_winner(ONE, &first, 1);
}

#[test]
fn override_recovery_changed_witness_is_refused() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    let id = fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    let (temp, backup) = fixture.witness(&id, ONE, 0);
    fs::write(&backup, b"tampered-witness").expect("tamper witness");

    let error = fixture.store.recover().expect_err("refuse");
    let message = format!("{error:#}");
    assert!(message.contains("CONFLICT"), "{message}");
    assert!(message.contains("changed override witness"), "{message}");
    assert_eq!(fs::read(fixture.dest(ONE)).expect("new bytes"), NEW);
    assert!(temp.exists() && backup.exists(), "witnesses preserved");
    assert_eq!(fixture.store.pending().expect("pending").len(), 1);
    fixture.assert_old_winner(ONE, &first, 1);
}

#[test]
fn override_recovery_keeps_unmanaged_neighbours_and_other_packages() {
    let mut fixture = Fixture::new();
    let other_archive = fixture.archive("other", &[("pack/other.bin", b"other-bytes")]);
    let other = fixture.install(&other_archive, &[("pack/other.bin", TWO)]);
    let first_archive = fixture.archive("first", &[("pack/one.bin", OLD)]);
    let first = fixture.install(&first_archive, &[("pack/one.bin", ONE)]);
    let second_archive = fixture.archive("second", &[("pack/one.bin", NEW)]);
    fixture.crash(&second_archive, &[("pack/one.bin", ONE)]);
    assert_eq!(fixture.store.recover().expect("recover"), 1);
    fixture.assert_old_owner(ONE, &first, 2);
    assert_eq!(
        fs::read(fixture.dest(TWO)).expect("other package"),
        b"other-bytes"
    );
    assert_eq!(
        fixture.winner(TWO),
        Some((other.id.clone(), other.files[0].blake3.clone()))
    );
    assert_eq!(
        fixture.store.installed(&fixture.game).expect("list").len(),
        2
    );
}
