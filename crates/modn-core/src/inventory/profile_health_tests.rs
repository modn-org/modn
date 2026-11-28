//! Tests of the read-only active-profile health report.
//!
//! Every test builds a disposable game tree, manager state and archive cache.
//! A helper writes the rows that a committed install leaves behind, so the
//! report can be checked without running an archive transaction.

use super::profile_health::{health, IssueKind, ProfileHealth};
use crate::state::Store;
use crate::transactions::ownership_key;
use rusqlite::params;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const GAME_ID: &str = "test-game";
const SHARED: &str = "addons/shared.plugin";
const OTHER: &str = "addons/other.plugin";

struct Fixture {
    _dir: TempDir,
    store: Store,
    game: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempdir().expect("fixture");
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("addons")).expect("addons");
        let game = game.canonicalize().expect("canonical game");
        let store = Store::open(&dir.path().join("state/state.sqlite")).expect("state");
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
        };
        fixture.profile("default", true);
        fixture
    }

    fn game_text(&self) -> String {
        self.game.to_string_lossy().into_owned()
    }

    fn profile(&self, name: &str, active: bool) -> String {
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

    fn member(&self, profile: &str, package: &str, priority: i64, enabled: bool) {
        self.store
            .connection
            .execute(
                "INSERT INTO profile_packages(profile_id,package_id,priority,enabled)
                 VALUES (?1,?2,?3,?4)",
                params![
                    format!("profile-{profile}"),
                    package,
                    priority,
                    i64::from(enabled)
                ],
            )
            .expect("membership");
    }

    /// Write one package, its claims and its cache entry.
    fn package(
        &self,
        name: &str,
        profile: &str,
        priority: i64,
        enabled: bool,
        files: &[(&str, &[u8])],
    ) {
        let archive = self.archive(name, files);
        let pin = blake3::hash(&fs::read(&archive).expect("archive bytes"))
            .to_hex()
            .to_string();
        self.store
            .connection
            .execute(
                "INSERT INTO managed_packages(id,game_path,archive_blake3) VALUES (?1,?2,?3)",
                params![name, self.game_text(), format!("blake3:{pin}")],
            )
            .expect("package");
        self.member(profile, name, priority, enabled);
        for (relative, bytes) in files {
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
                        blake3::hash(bytes).to_hex().to_string(),
                        i64::try_from(bytes.len()).expect("size")
                    ],
                )
                .expect("claim");
        }
    }

    /// Write the managed file rows and game bytes that make `name` the winner.
    fn materialize(&self, profile: &str, name: &str, files: &[(&str, &[u8])]) {
        for (relative, bytes) in files {
            let target = self.game.join(relative);
            fs::create_dir_all(target.parent().expect("parent")).expect("parent");
            fs::write(&target, bytes).expect("bytes");
            self.store
                .connection
                .execute(
                    "INSERT OR REPLACE INTO managed_files
                     (game_path,root,path_key,relative_path,package_id,blake3,bytes,
                      profile_id,archive_entry)
                     VALUES (?1,'game',?2,?3,?4,?5,?6,?7,?3)",
                    params![
                        self.game_text(),
                        ownership_key(relative),
                        relative,
                        name,
                        blake3::hash(bytes).to_hex().to_string(),
                        i64::try_from(bytes.len()).expect("size"),
                        format!("profile-{profile}")
                    ],
                )
                .expect("winner");
        }
    }

    fn install(&self, name: &str, profile: &str, priority: i64, files: &[(&str, &[u8])]) {
        self.package(name, profile, priority, true, files);
        self.materialize(profile, name, files);
    }

    fn archive(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = self._dir.path().join(format!("archives/{name}.zip"));
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

    fn remove_cached_archive(&self, name: &str) {
        fs::remove_file(self.cached_archive(name)).expect("remove archive");
    }

    fn cached_archive(&self, name: &str) -> PathBuf {
        self.store
            .state_root
            .join("cache")
            .join(format!("test-game#{name}#1.0#0000000.zip"))
    }

    fn report(&self) -> ProfileHealth {
        health(&self.store, &self.game)
            .expect("health")
            .expect("active profile")
    }

    fn reorder(&self, profile: &str, package: &str, priority: i64) {
        self.store
            .connection
            .execute(
                "UPDATE profile_packages SET priority=?1 WHERE profile_id=?2 AND package_id=?3",
                params![priority, format!("profile-{profile}"), package],
            )
            .expect("priority");
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn collect(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(directory).expect("directory entries") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        if entry.file_type().expect("entry type").is_dir() {
            collect(root, &path, files);
        } else {
            let relative = path.strip_prefix(root).expect("relative path").to_owned();
            files.insert(relative, fs::read(&path).expect("file bytes"));
        }
    }
}

#[test]
fn profile_health_without_overlap_is_consistent() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(OTHER, b"first")]);
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);

    let report = fixture.report();

    assert_eq!(report.name, "default");
    assert!(report.active);
    assert_eq!(report.packages, 2);
    assert_eq!(report.overlapping_files, 0);
    assert!(report.consistent);
    assert!(report.issues.is_empty());
}

#[test]
fn profile_health_without_an_active_profile_is_absent() {
    let fixture = Fixture::new();
    fixture
        .store
        .connection
        .execute("UPDATE profiles SET active=0", [])
        .expect("deactivate");

    assert!(health(&fixture.store, &fixture.game)
        .expect("health")
        .is_none());
}

#[test]
fn profile_health_reports_a_winner_mismatch() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(SHARED, b"first")]);
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);
    fixture.reorder("default", "first", 2);

    let report = fixture.report();

    assert_eq!(report.overlapping_files, 1);
    assert!(!report.consistent);
    assert_eq!(report.issues.len(), 1);
    let issue = &report.issues[0];
    assert_eq!(issue.kind, IssueKind::WinnerMismatch);
    assert_eq!(issue.path, SHARED);
    assert_eq!(issue.package, "first");
    assert_eq!(issue.advice, "modn profile apply --yes");
    assert!(issue.detail.contains("second"));
}

#[test]
fn profile_health_reports_a_missing_shadowed_archive() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(SHARED, b"first")]);
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);
    fixture.remove_cached_archive("first");

    let report = fixture.report();

    assert!(!report.consistent);
    assert_eq!(
        report.issues.len(),
        1,
        "only the shadowed claim needs the archive: {:?}",
        report.issues
    );
    let issue = &report.issues[0];
    assert_eq!(issue.kind, IssueKind::ShadowedClaimUnrecoverable);
    assert_eq!(issue.package, "first");
    assert_eq!(issue.path, SHARED);
    assert_eq!(issue.advice, "modn cache show");
}

#[test]
fn profile_health_reports_a_changed_archive_digest() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(SHARED, b"first")]);
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);
    fs::write(fixture.cached_archive("first"), b"changed archive bytes").expect("change");

    let report = fixture.report();

    assert!(!report.consistent);
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].kind, IssueKind::ShadowedClaimUnrecoverable);
    assert_eq!(report.issues[0].package, "first");
}

#[test]
fn profile_health_reports_another_profiles_missing_archive() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(OTHER, b"first")]);
    fixture.profile("spare", false);
    fixture.package("second", "spare", 0, true, &[(SHARED, b"second")]);
    fixture.remove_cached_archive("second");

    let report = fixture.report();

    assert!(!report.consistent);
    assert_eq!(report.issues.len(), 1);
    let issue = &report.issues[0];
    assert_eq!(issue.kind, IssueKind::OtherProfileArchiveMissing);
    assert_eq!(issue.package, "second");
    assert_eq!(issue.path, SHARED);
    assert!(issue.detail.contains("spare"));
}

#[test]
fn profile_health_ignores_the_claims_of_a_disabled_package() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(SHARED, b"first")]);
    fixture.package("second", "default", 1, false, &[(SHARED, b"second")]);
    fixture.remove_cached_archive("second");

    let report = fixture.report();

    assert!(report.consistent);
    assert!(report.issues.is_empty());
    assert_eq!(report.packages, 2);
    assert_eq!(report.overlapping_files, 0);
}

#[test]
fn profile_health_reads_without_changing_state_or_files() {
    let fixture = Fixture::new();
    fixture.install("first", "default", 0, &[(SHARED, b"first")]);
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);
    fixture.reorder("default", "first", 2);
    let game_before = snapshot(&fixture.game);
    let state_before = snapshot(&fixture.store.state_root);

    let first = fixture.report();
    let second = fixture.report();

    assert_eq!(first, second);
    assert_eq!(snapshot(&fixture.game), game_before);
    assert_eq!(snapshot(&fixture.store.state_root), state_before);
}

#[test]
fn profile_health_issue_order_is_deterministic() {
    let fixture = Fixture::new();
    fixture.profile("spare", false);
    fixture.install("winner", "default", 0, &[(SHARED, b"winner")]);
    fixture.install("loser", "default", 1, &[(SHARED, b"loser")]);
    fixture.package("switch", "spare", 0, true, &[(OTHER, b"switch")]);
    fixture.remove_cached_archive("switch");
    fixture.remove_cached_archive("loser");
    fixture.reorder("default", "winner", 2);

    let first = fixture.report();
    let second = fixture.report();
    let kinds: Vec<IssueKind> = first.issues.iter().map(|issue| issue.kind).collect();

    assert_eq!(first, second);
    assert_eq!(
        kinds,
        [
            IssueKind::WinnerMismatch,
            IssueKind::ShadowedClaimUnrecoverable,
            IssueKind::OtherProfileArchiveMissing,
        ]
    );
    assert_eq!(first.issues[0].path, SHARED);
    assert_eq!(first.issues[1].path, SHARED);
    assert_eq!(first.issues[2].path, OTHER);
}

#[test]
fn profile_health_counts_memberships_and_overlaps() {
    let fixture = Fixture::new();
    fixture.install(
        "first",
        "default",
        0,
        &[(SHARED, b"first"), (OTHER, b"first")],
    );
    fixture.install("second", "default", 1, &[(SHARED, b"second")]);
    fixture.package("third", "default", 2, false, &[(OTHER, b"third")]);

    let report = fixture.report();

    assert_eq!(report.packages, 3);
    assert_eq!(report.overlapping_files, 1);
    assert!(report.consistent);
}
