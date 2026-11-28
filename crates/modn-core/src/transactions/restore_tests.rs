//! Uninstall restores the next claimant's verified bytes from the archive cache.
//!
//! Each test uses a disposable game tree and manager state. The cache is seeded
//! with the exact archive bytes so the restore path can verify the pinned
//! digest, the member digest and the journal witnesses.

use super::cleanup::RemovalMode;
use super::install::Fault;
use super::journal::lock_store;
use super::ManagedPackage;
use crate::archive::inspect;
use crate::error::{code_of, ErrorCode};
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

pub(super) const SHARED: &str = "addons/shared.plugin";
pub(super) const UNMANAGED: &str = "addons/user-notes.txt";
pub(super) const ENTRY: &str = "pack/shared.plugin";

pub(super) struct Fixture {
    _dir: TempDir,
    pub(super) store: Store,
    pub(super) game: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
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

    fn archive(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self._dir.path().join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(File::create(&path).expect("zip"));
        zip.start_file(ENTRY, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(bytes).expect("bytes");
        zip.finish().expect("finish");
        path
    }

    fn plan(&self, archive: &Path, destination: &str) -> InstallPlan {
        let checked = inspect(archive).expect("inspect");
        InstallPlan {
            archive_blake3: checked.blake3,
            game_installation: self.game.clone(),
            status: PlanStatus::Reviewable,
            metadata_only: true,
            operations: checked
                .files
                .into_iter()
                .map(|entry| PlannedFile {
                    source: entry.entry,
                    destination: destination.to_owned(),
                    bytes: entry.bytes,
                    kind: "file-copy".into(),
                    owned_directory: None,
                    root: "game".into(),
                })
                .collect(),
            blockers: Vec::new(),
            skipped_generated_files: Vec::new(),
            content_base: None,
            content_root: None,
            overrides: Vec::new(),
        }
    }

    pub(super) fn install(&mut self, name: &str, bytes: &[u8]) -> ManagedPackage {
        let archive = self.archive(name, bytes);
        let plan = self.plan(&archive, SHARED);
        let game = self.game.clone();
        let package = self
            .store
            .install_operations_zip_with_permissions(&game, &archive, &plan, &[])
            .expect("install");
        self.seed_cache(name, &archive);
        package
    }

    /// Put the exact archive bytes in the manager profile cache.
    fn seed_cache(&self, name: &str, archive: &Path) {
        let cache = self.store.state_root.join("cache");
        fs::create_dir_all(&cache).expect("cache");
        fs::copy(
            archive,
            cache.join(format!("test-game#{name}#1.0#0000000.zip")),
        )
        .expect("cache entry");
    }

    fn uninstall(&mut self, package_id: &str) -> anyhow::Result<()> {
        let game = self.game.clone();
        self.store.uninstall(&game, package_id)
    }

    pub(super) fn path(&self) -> PathBuf {
        self.game.join(SHARED)
    }

    pub(super) fn bytes(&self) -> Vec<u8> {
        fs::read(self.path()).expect("bytes")
    }

    pub(super) fn winner(&self) -> Option<String> {
        self.store
            .connection
            .query_row(
                "SELECT package_id FROM managed_files WHERE relative_path=?1",
                [SHARED],
                |row| row.get(0),
            )
            .ok()
    }

    fn count(&self, sql: &str) -> i64 {
        self.store
            .connection
            .query_row(sql, [], |row| row.get(0))
            .expect("count")
    }

    pub(super) fn installed(&self) -> usize {
        self.store.installed(&self.game).expect("installed").len()
    }

    pub(super) fn pending(&self) -> usize {
        self.store.pending().expect("pending").len()
    }

    pub(super) fn crash_uninstall(&mut self, package_id: &str) -> String {
        let _guard = lock_store(&self.store).expect("lock");
        let result = self.store.uninstall_with_mode_locked(
            &self.game,
            package_id,
            RemovalMode::Strict,
            Some(Fault::CrashAfter(1)),
        );
        assert!(result.is_err(), "the injected stop must fail");
        let pending = self.store.pending().expect("pending");
        assert_eq!(pending.len(), 1);
        pending[0].id.clone()
    }

    fn unmanaged_intact(&self) {
        assert_eq!(
            fs::read(self.game.join(UNMANAGED)).expect("unmanaged"),
            b"user-bytes"
        );
    }
}

#[test]
fn restore_uninstall_winner_uses_next_claimant_bytes() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    assert_eq!(fixture.bytes(), b"second-bytes");

    fixture.uninstall(&second.id).expect("uninstall winner");

    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 1);
    assert_eq!(fixture.installed(), 1);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM profile_packages"), 1);
    assert_eq!(fixture.pending(), 0);
    fixture.unmanaged_intact();
}

#[test]
fn restore_chain_uses_priority_order() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");

    fixture.uninstall(&third.id).expect("uninstall third");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));

    fixture.uninstall(&second.id).expect("uninstall second");
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
    assert_eq!(fixture.installed(), 1);
}

#[test]
fn restore_skips_a_disabled_claimant() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let third = fixture.install("third", b"third-bytes");
    crate::profiles::set_enabled(
        &mut fixture.store,
        &fixture.game,
        "default",
        &second.id,
        false,
    )
    .expect("disable second");

    fixture.uninstall(&third.id).expect("uninstall third");
    assert_eq!(fixture.bytes(), b"first-bytes");
    assert_eq!(fixture.winner(), Some(first.id.clone()));
}

#[test]
fn restore_missing_archive_refuses_without_change() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fs::remove_dir_all(fixture.store.state_root.join("cache")).expect("remove cache");

    let error = fixture.uninstall(&second.id).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    let message = format!("{error:#}");
    assert!(message.starts_with("PROFILE_ARCHIVE_MISSING:"), "{message}");
    assert!(message.contains("modn cache show"), "{message}");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.installed(), 2);
    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_archive_entry_mismatch_refuses_without_change() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fixture
        .store
        .connection
        .execute(
            "UPDATE file_claims SET blake3=?1 WHERE relative_path=?2 AND package_id<>?3",
            rusqlite::params![digest_bytes(b"wrong"), SHARED, second.id],
        )
        .expect("tamper claim");

    let error = fixture.uninstall(&second.id).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileArchiveMissing));
    let message = format!("{error:#}");
    assert!(message.contains("archive entry"), "{message}");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_drifted_winner_refuses_with_profile_drift() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    fs::write(fixture.path(), b"user-edit").expect("edit");

    let error = fixture.uninstall(&second.id).expect_err("refuse");
    assert_eq!(code_of(&error), Some(ErrorCode::ProfileDrift));
    let message = format!("{error:#}");
    assert!(message.starts_with("PROFILE_DRIFT:"), "{message}");
    assert!(message.contains("modn doctor"), "{message}");
    assert_eq!(fixture.bytes(), b"user-edit");
    assert_eq!(fixture.installed(), 2);
    assert_eq!(fixture.pending(), 0);
}

#[test]
fn restore_leaves_unmanaged_files_untouched() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    fixture.uninstall(&second.id).expect("uninstall winner");
    fixture.unmanaged_intact();
    assert_eq!(fixture.bytes(), b"first-bytes");
}

#[test]
fn restore_without_claimant_removes_the_path() {
    let mut fixture = Fixture::new();
    let only = fixture.install("only", b"only-bytes");

    fixture.uninstall(&only.id).expect("uninstall");
    assert!(!fixture.path().exists());
    assert_eq!(fixture.count("SELECT COUNT(*) FROM managed_files"), 0);
    fixture.unmanaged_intact();
}

#[test]
fn restore_shadowed_only_uninstall_keeps_winner_bytes() {
    let mut fixture = Fixture::new();
    let first = fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");

    fixture.uninstall(&first.id).expect("uninstall shadowed");
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.installed(), 1);
}

fn digest_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[test]
fn restore_undo_of_install_refuses_a_winning_package() {
    let mut fixture = Fixture::new();
    fixture.install("first", b"first-bytes");
    let second = fixture.install("second", b"second-bytes");
    let allowed = std::collections::BTreeSet::from([second.id.clone()]);
    let _guard = lock_store(&fixture.store).expect("lock");

    let error = fixture
        .store
        .withdraw_member_locked(&fixture.game, &second.id, &allowed)
        .expect_err("refuse undo");
    let message = format!("{error:#}");
    assert!(message.contains("CONFLICT"), "{message}");
    assert!(
        message.contains("cannot restore the next claimant"),
        "{message}"
    );
    assert_eq!(fixture.bytes(), b"second-bytes");
    assert_eq!(fixture.winner(), Some(second.id.clone()));
    assert_eq!(fixture.installed(), 2);
}
