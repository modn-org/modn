//! The diff of a profile switch: removals, writes, winners and properties.
//!
//! Every test uses a disposable game tree, manager state and cache. A helper
//! writes the rows an install would commit, so the plan can be checked without
//! running an archive transaction.

use super::{plan_profile_switch, PathAction, ProfileSwitchPlan};
use crate::state::Store;
use crate::transactions::ownership_key;
use rusqlite::params;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

pub(super) const GAME_ID: &str = "test-game";
pub(super) const SHARED: &str = "addons/shared.plugin";

pub(super) struct Fixture {
    _dir: TempDir,
    pub(super) store: Store,
    pub(super) game: PathBuf,
    state_path: PathBuf,
    next: usize,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let dir = tempdir().expect("fixture");
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("addons")).expect("addons");
        let game = game.canonicalize().expect("canonical game");
        let state_path = dir.path().join("state/state.sqlite");
        let store = Store::open(&state_path).expect("state");
        store
            .connection
            .execute(
                "INSERT INTO game_installations(game_id,path,source) VALUES (?1,?2,'manual')",
                params![GAME_ID, game.to_string_lossy().as_ref()],
            )
            .expect("installation");
        let fixture = Self {
            _dir: dir,
            store,
            game,
            state_path,
            next: 0,
        };
        fixture.profile("default", true);
        fixture
    }

    pub(super) fn game_text(&self) -> String {
        self.game.to_string_lossy().into_owned()
    }

    pub(super) fn profile(&self, name: &str, active: bool) -> String {
        let id = format!("profile-{name}");
        self.store
            .connection
            .execute(
                "INSERT INTO profiles(id,game_path,name,active) VALUES (?1,?2,?3,?4)",
                params![id, self.game_text(), name, i64::from(active)],
            )
            .expect("profile");
        id
    }

    pub(super) fn member(&self, profile: &str, package: &str, priority: i64, enabled: bool) {
        let profile_id = format!("profile-{profile}");
        self.store
            .connection
            .execute(
                "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
                 VALUES (?1,?2,?3,?4)",
                params![profile_id, package, priority, i64::from(enabled)],
            )
            .expect("membership");
    }

    pub(super) fn package(
        &mut self,
        name: &str,
        profile: &str,
        priority: i64,
        enabled: bool,
        files: &[(&str, &[u8])],
    ) {
        let archive = self.archive(name, files);
        let pin = super::super::journal::digest(&archive).expect("pin");
        self.store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,?3)",
                params![name, self.game_text(), format!("blake3:{pin}")],
            )
            .expect("package");
        self.member(profile, name, priority, enabled);
        for (relative, bytes) in files {
            let blake3 = blake3::hash(bytes).to_hex().to_string();
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
                        i64::try_from(bytes.len()).expect("size")
                    ],
                )
                .expect("claim");
        }
    }

    fn archive(&mut self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        self.next += 1;
        let path = self
            ._dir
            .path()
            .join(format!("archives/{name}-{}.zip", self.next));
        fs::create_dir_all(path.parent().expect("parent")).expect("archives");
        let mut zip = zip::ZipWriter::new(File::create(&path).expect("zip"));
        for (entry, bytes) in files {
            zip.start_file(*entry, SimpleFileOptions::default())
                .expect("entry");
            zip.write_all(bytes).expect("bytes");
        }
        zip.finish().expect("finish");
        let cache = self.store.state_root.join("cache");
        fs::create_dir_all(&cache).expect("cache");
        fs::copy(
            &path,
            cache.join(format!("test-game#{name}#1.0#0000000.zip")),
        )
        .expect("cache entry");
        path
    }

    /// Commit the winner rows and the game bytes of one package.
    fn materialize(&self, profile: &str, package: &str, files: &[(&str, &[u8])]) {
        let profile_id = format!("profile-{profile}");
        for (relative, bytes) in files {
            let target = self.game.join(relative);
            fs::create_dir_all(target.parent().expect("parent")).expect("parent");
            fs::write(&target, bytes).expect("bytes");
            self.store
                .connection
                .execute(
                    "INSERT INTO managed_files
                     (game_path,root,path_key,relative_path,package_id,blake3,bytes,
                      profile_id,archive_entry)
                     VALUES (?1,'game',?2,?3,?4,?5,?6,?7,?3)",
                    params![
                        self.game_text(),
                        ownership_key(relative),
                        relative,
                        package,
                        blake3::hash(bytes).to_hex().to_string(),
                        i64::try_from(bytes.len()).expect("size"),
                        profile_id
                    ],
                )
                .expect("winner");
        }
    }

    pub(super) fn install(
        &mut self,
        name: &str,
        profile: &str,
        priority: i64,
        files: &[(&str, &[u8])],
    ) {
        self.package(name, profile, priority, true, files);
        self.materialize(profile, name, files);
    }

    pub(super) fn plan(&self, target: &str) -> anyhow::Result<ProfileSwitchPlan> {
        plan_profile_switch(&self.store, &self.game, target)
    }

    pub(super) fn remove_cache(&self) {
        fs::remove_dir_all(self.store.state_root.join("cache")).expect("remove cache");
    }

    /// Change a claim digest so its verified archive member cannot match.
    pub(super) fn tamper_claim(&self, package: &str, relative: &str) {
        self.store
            .connection
            .execute(
                "UPDATE file_claims SET blake3=?1 WHERE package_id=?2 AND relative_path=?3",
                params![
                    blake3::hash(b"wrong").to_hex().to_string(),
                    package,
                    relative
                ],
            )
            .expect("tamper claim");
    }

    fn tree(&self) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(base).expect("read dir") {
                let entry = entry.expect("entry");
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    let key = path
                        .strip_prefix(root)
                        .expect("relative")
                        .to_string_lossy()
                        .into_owned();
                    out.insert(key, fs::read(&path).expect("bytes"));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.game, &self.game, &mut out);
        out
    }

    fn state_rows(&self) -> Vec<(String, i64)> {
        [
            "profiles",
            "profile_packages",
            "file_claims",
            "managed_files",
            "pending_transactions",
        ]
        .into_iter()
        .map(|table| {
            let count = self
                .store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count");
            (table.to_owned(), count)
        })
        .collect()
    }
}

#[test]
fn profile_switch_disjoint_profiles_remove_and_write_all() {
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

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.remove.len(), 1);
    assert_eq!(plan.remove[0].path, "addons/first.plugin");
    assert_eq!(plan.remove[0].action, PathAction::Remove);
    assert_eq!(plan.remove[0].package.as_deref(), Some("first"));
    assert_eq!(plan.write.len(), 1);
    assert_eq!(plan.write[0].path, "addons/second.plugin");
    assert_eq!(plan.write[0].action, PathAction::Write);
    assert_eq!(plan.write[0].package.as_deref(), Some("second"));
    assert_eq!(plan.unchanged, 0);
    assert_eq!(plan.bytes_to_write, 6);
    assert_eq!(plan.entering, ["second"]);
    assert_eq!(plan.leaving, ["first"]);
    assert_eq!(plan.from, "default");
    assert_eq!(plan.to, "target");
    assert_eq!(plan.game, GAME_ID);
}

#[test]
fn profile_switch_overlapping_package_is_unchanged() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("shared", "default", 0, &[(SHARED, b"bytes")]);
    fixture.member("target", "shared", 0, true);

    let plan = fixture.plan("target").expect("plan");

    assert!(plan.remove.is_empty());
    assert!(plan.write.is_empty());
    assert_eq!(plan.unchanged, 1);
    assert_eq!(plan.unchanged_paths.len(), 1);
    assert_eq!(plan.unchanged_paths[0].path, SHARED);
    assert!(plan.entering.is_empty());
    assert!(plan.leaving.is_empty());
    assert_eq!(plan.bytes_to_write, 0);
}

#[test]
fn profile_switch_different_winners_replace_with_owner() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"first-bytes")]);
    fixture.package("second", "target", 0, true, &[(SHARED, b"second-bytes")]);

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.write.len(), 1);
    let write = &plan.write[0];
    assert_eq!(write.path, SHARED);
    assert_eq!(write.action, PathAction::Replace);
    assert_eq!(write.package.as_deref(), Some("second"));
    assert_eq!(write.replaces.as_deref(), Some("first"));
    assert_eq!(write.bytes, Some(12));
    assert_eq!(
        write.blake3.as_deref(),
        Some(blake3::hash(b"second-bytes").to_hex().as_str())
    );
    assert_eq!(plan.bytes_to_write, 12);
    assert!(plan.remove.is_empty());
}

#[test]
fn profile_switch_entering_and_leaving_packages() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 1, &[("addons/one.plugin", b"one")]);
    fixture.install("kept", "default", 0, &[("addons/kept.plugin", b"kept")]);
    fixture.member("target", "kept", 0, true);
    fixture.package(
        "third",
        "target",
        1,
        true,
        &[("addons/three.plugin", b"three")],
    );

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.entering, ["third"]);
    assert_eq!(plan.leaving, ["first"]);
}

#[test]
fn profile_switch_priority_decides_target_winner() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package("low", "target", 0, true, &[(SHARED, b"low-bytes")]);
    fixture.package("high", "target", 1, true, &[(SHARED, b"high-bytes")]);

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.write.len(), 1);
    assert_eq!(plan.write[0].package.as_deref(), Some("high"));
    assert_eq!(plan.write[0].replaces.as_deref(), Some("first"));
    assert_eq!(plan.write[0].bytes, Some(10));
}

#[test]
fn profile_switch_disabled_package_in_target_does_not_claim() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package(
        "disabled",
        "target",
        1,
        false,
        &[(SHARED, b"disabled-bytes")],
    );

    let plan = fixture.plan("target").expect("plan");

    assert!(plan.write.is_empty());
    assert_eq!(plan.remove.len(), 1);
    assert_eq!(plan.remove[0].path, SHARED);
    assert!(plan.entering.is_empty());
    assert_eq!(plan.leaving, ["first"]);
}

#[test]
fn profile_switch_empty_target_removes_everything() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[("addons/a.plugin", b"a"), ("addons/b.plugin", b"b")],
    );

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.remove.len(), 2);
    assert!(plan.write.is_empty());
    assert_eq!(plan.unchanged, 0);
    assert_eq!(plan.bytes_to_write, 0);
    assert_eq!(plan.leaving, ["first"]);
}

#[test]
fn profile_switch_plan_is_sorted_and_deterministic() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install(
        "first",
        "default",
        0,
        &[
            ("addons/z.plugin", b"z"),
            ("addons/a.plugin", b"a"),
            ("addons/m.plugin", b"m"),
        ],
    );

    let first = fixture.plan("target").expect("plan");
    let second = fixture.plan("target").expect("plan");
    let paths: Vec<&str> = first.remove.iter().map(|path| path.path.as_str()).collect();

    assert_eq!(
        paths,
        ["addons/a.plugin", "addons/m.plugin", "addons/z.plugin"]
    );
    assert_eq!(first, second);
}

#[test]
fn profile_switch_plan_changes_no_game_byte_and_no_state_row() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"current")]);
    fixture.package("second", "target", 0, true, &[(SHARED, b"second-bytes")]);
    let tree = fixture.tree();
    let rows = fixture.state_rows();
    let state = fs::read(&fixture.state_path).expect("state bytes");

    let plan = fixture.plan("target").expect("plan");

    assert_eq!(plan.write.len(), 1);
    assert_eq!(fixture.tree(), tree);
    assert_eq!(fixture.state_rows(), rows);
    assert_eq!(fs::read(&fixture.state_path).expect("state bytes"), state);
}

#[test]
fn profile_switch_json_shape_is_stable() {
    let mut fixture = Fixture::new();
    fixture.profile("target", false);
    fixture.install("first", "default", 0, &[(SHARED, b"first-bytes")]);
    fixture.package("second", "target", 0, true, &[(SHARED, b"second-bytes")]);

    let plan = fixture.plan("target").expect("plan");
    let value = serde_json::to_value(&plan).expect("json");
    let object = value.as_object().expect("object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();

    assert_eq!(
        keys,
        [
            "bytes_to_write",
            "entering",
            "from",
            "game",
            "leaving",
            "path",
            "remove",
            "to",
            "unchanged",
            "unchanged_paths",
            "write",
        ]
    );
    let write = object["write"][0].as_object().expect("write object");
    let mut write_keys: Vec<&str> = write.keys().map(String::as_str).collect();
    write_keys.sort_unstable();
    assert_eq!(
        write_keys,
        ["action", "blake3", "bytes", "package", "path", "replaces", "root"]
    );
    assert_eq!(object["write"][0]["action"], "replace");
    assert_eq!(object["write"][0]["replaces"], "first");
    assert_eq!(object["unchanged"], 0);
}
