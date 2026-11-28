//! Seven-zip archives in uninstall restore, profile toggle and profile apply.
//!
//! Uninstall, toggle and apply stage every needed archive member into one
//! transaction directory and place it by ordinal. These tests use real 7z
//! archives so several members share that directory.

use super::journal::digest;
use crate::profiles::Placement;
use crate::state::Store;
use crate::transactions::{ownership_key, ProfileToggle, ToggleAction};
use rusqlite::params;
use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use tempfile::{tempdir, TempDir};

pub(super) struct Fixture {
    _dir: TempDir,
    pub(super) store: Store,
    pub(super) game: PathBuf,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let dir = tempdir().expect("fixture");
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("addons")).expect("addons");
        let game = game.canonicalize().expect("canonical game");
        let store = Store::open(&dir.path().join("state/state.sqlite")).expect("state");
        let game_text = game.to_string_lossy().into_owned();
        store
            .connection
            .execute(
                "INSERT INTO game_installations(game_id,path,source) VALUES ('test-game',?1,'manual')",
                params![game_text],
            )
            .expect("installation");
        store
            .connection
            .execute(
                "INSERT INTO profiles(id,game_path,name,active)
                 VALUES ('profile-default',?1,'default',1)",
                params![game_text],
            )
            .expect("profile");
        Self {
            _dir: dir,
            store,
            game,
        }
    }

    pub(super) fn game_text(&self) -> String {
        self.game.to_string_lossy().into_owned()
    }

    /// Add package `name` to the active profile with a 7z archive in the cache.
    ///
    /// With `winner` set, the package also holds the current bytes and winner
    /// rows of every file, as a committed install would.
    pub(super) fn package(
        &mut self,
        name: &str,
        priority: i64,
        enabled: bool,
        files: &[(&str, &[u8])],
        winner: bool,
    ) {
        let archive = self.sevenz(name, files);
        let pin = digest(&archive).expect("7z pin");
        self.store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,?3)",
                params![name, self.game_text(), format!("blake3:{pin}")],
            )
            .expect("package");
        self.store
            .connection
            .execute(
                "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
                 VALUES ('profile-default',?1,?2,?3)",
                params![name, priority, i64::from(enabled)],
            )
            .expect("membership");
        for (relative, bytes) in files {
            let blake3 = blake3::hash(bytes).to_hex().to_string();
            let size = i64::try_from(bytes.len()).expect("size");
            self.store
                .connection
                .execute(
                    "INSERT INTO file_claims
                     (package_id,game_path,root,path_key,relative_path,archive_entry,blake3,bytes)
                     VALUES (?1,?2,'game',?3,?4,?4,?5,?6)",
                    params![
                        name,
                        self.game_text(),
                        ownership_key(relative),
                        relative,
                        blake3,
                        size
                    ],
                )
                .expect("claim");
            if winner {
                let target = self.game.join(relative);
                fs::create_dir_all(target.parent().expect("parent")).expect("parent");
                fs::write(&target, bytes).expect("bytes");
                self.store
                    .connection
                    .execute(
                        "INSERT INTO managed_files
                         (game_path,root,path_key,relative_path,package_id,blake3,bytes,
                          profile_id,archive_entry)
                         VALUES (?1,'game',?2,?3,?4,?5,?6,'profile-default',?3)",
                        params![
                            self.game_text(),
                            ownership_key(relative),
                            relative,
                            name,
                            blake3,
                            size
                        ],
                    )
                    .expect("winner");
            }
        }
    }

    /// Write a 7z archive of `files` into the manager cache.
    fn sevenz(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let cache = self.store.state_root.join("cache");
        fs::create_dir_all(&cache).expect("cache");
        let path = cache.join(format!("test-game#{name}#1.0#0000000.7z"));
        let mut writer = ArchiveWriter::create(&path).expect("7z writer");
        for (entry, bytes) in files {
            writer
                .push_archive_entry(ArchiveEntry::new_file(entry), Some(Cursor::new(*bytes)))
                .expect("7z member");
        }
        writer.finish().expect("complete 7z");
        path
    }

    pub(super) fn bytes(&self, relative: &str) -> Vec<u8> {
        fs::read(self.game.join(relative)).expect("bytes")
    }

    pub(super) fn winner(&self, relative: &str) -> Option<String> {
        self.store
            .connection
            .query_row(
                "SELECT package_id FROM managed_files WHERE relative_path=?1",
                [relative],
                |row| row.get(0),
            )
            .ok()
    }

    pub(super) fn pending(&self) -> usize {
        self.store.pending().expect("pending").len()
    }

    pub(super) fn uninstall(&mut self, package: &str) -> anyhow::Result<()> {
        let game = self.game.clone();
        self.store.uninstall(&game, package)
    }

    pub(super) fn toggle(&mut self, package: &str, enabled: bool) -> anyhow::Result<ProfileToggle> {
        let game = self.game.clone();
        self.store
            .profile_toggle(&game, "default", package, enabled)
    }

    pub(super) fn apply(&mut self) -> anyhow::Result<()> {
        let game = self.game.clone();
        self.store.apply_profile(&game, Some("default")).map(|_| ())
    }

    pub(super) fn move_to_rank(&mut self, package: &str, rank: i64) -> anyhow::Result<()> {
        let game = self.game.clone();
        self.store
            .move_priority(&game, "default", package, &Placement::Rank(rank))
            .map(|_| ())
    }
}

fn two_files() -> &'static [(&'static str, &'static [u8])] {
    &[
        ("addons/one.plugin", b"first-one"),
        ("addons/two.plugin", b"first-two"),
    ]
}

fn two_other_files() -> &'static [(&'static str, &'static [u8])] {
    &[
        ("addons/one.plugin", b"second-one"),
        ("addons/two.plugin", b"second-two"),
    ]
}

fn staging_dirs(store: &Store) -> Vec<String> {
    fs::read_dir(&store.state_root)
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

fn assert_no_staging(store: &Store) {
    assert!(
        staging_dirs(store).is_empty(),
        "no transaction or scratch directory stays"
    );
}

#[test]
fn restore_uninstall_restores_two_files_from_one_sevenz_archive() {
    let mut fixture = Fixture::new();
    fixture.package("first", 0, true, two_files(), false);
    fixture.package("second", 1, true, two_other_files(), true);
    assert_eq!(fixture.bytes("addons/one.plugin"), b"second-one");

    fixture.uninstall("second").expect("uninstall winner");

    assert_eq!(fixture.bytes("addons/one.plugin"), b"first-one");
    assert_eq!(fixture.bytes("addons/two.plugin"), b"first-two");
    assert_eq!(
        fixture.winner("addons/one.plugin").as_deref(),
        Some("first")
    );
    assert_eq!(
        fixture.winner("addons/two.plugin").as_deref(),
        Some("first")
    );
    assert_eq!(fixture.pending(), 0);
    assert_no_staging(&fixture.store);
}

#[test]
fn profile_toggle_enable_restores_two_sevenz_members() {
    let mut fixture = Fixture::new();
    fixture.package("incumbent", 0, true, two_files(), true);
    fixture.package("returning", 1, false, two_other_files(), false);

    let report = fixture
        .toggle("returning", true)
        .expect("enable two 7z files");

    assert_eq!(report.changes.len(), 2);
    assert!(report
        .changes
        .iter()
        .all(|change| change.action == ToggleAction::Replace));
    assert_eq!(fixture.bytes("addons/one.plugin"), b"second-one");
    assert_eq!(fixture.bytes("addons/two.plugin"), b"second-two");
    assert_eq!(
        fixture.winner("addons/one.plugin").as_deref(),
        Some("returning")
    );
    assert_eq!(fixture.pending(), 0);
    assert_no_staging(&fixture.store);
}

#[test]
fn profile_apply_restores_two_sevenz_members() {
    let mut fixture = Fixture::new();
    fixture.package("low", 0, true, two_files(), false);
    fixture.package("high", 1, true, two_other_files(), true);
    assert_eq!(fixture.bytes("addons/one.plugin"), b"second-one");

    fixture
        .move_to_rank("high", 2)
        .expect("move the winner to rank 2");
    fixture.apply().expect("apply two 7z files");

    assert_eq!(fixture.bytes("addons/one.plugin"), b"first-one");
    assert_eq!(fixture.bytes("addons/two.plugin"), b"first-two");
    assert_eq!(fixture.winner("addons/one.plugin").as_deref(), Some("low"));
    assert_eq!(fixture.winner("addons/two.plugin").as_deref(), Some("low"));
    assert_eq!(fixture.pending(), 0);
    assert_no_staging(&fixture.store);
}
