//! Installing over a lower-priority managed file, and the interim refusals.

use super::catalog::{CatalogReference, CatalogRelease};
use super::{ManagedPackage, NativeInjectorPolicy};
use crate::archive::inspect;
use crate::planning::{InstallPlan, PlanStatus, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

const SHARED: &str = "addons/shared.plugin";

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
        zip.start_file("pack/shared.plugin", SimpleFileOptions::default())
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

    fn install(&mut self, archive: &Path) -> anyhow::Result<ManagedPackage> {
        let plan = self.plan(archive, SHARED);
        let game = self.game.clone();
        self.store
            .install_operations_zip_with_permissions(&game, archive, &plan, &[])
    }

    fn install_release(
        &mut self,
        archive: &Path,
        release: &CatalogRelease,
    ) -> anyhow::Result<ManagedPackage> {
        let plan = self.plan(archive, SHARED);
        let game = self.game.clone();
        self.store.install_catalog_release_with_permissions(
            &game,
            archive,
            &plan,
            release,
            NativeInjectorPolicy::Refuse,
            &[],
        )
    }

    fn path(&self) -> PathBuf {
        self.game.join(SHARED)
    }

    fn count(&self, sql: &str) -> i64 {
        self.store
            .connection
            .query_row(sql, [], |row| row.get(0))
            .expect("count")
    }

    fn winner(&self) -> Option<String> {
        self.store
            .connection
            .query_row(
                "SELECT package_id FROM managed_files WHERE relative_path=?1",
                [SHARED],
                |row| row.get(0),
            )
            .ok()
    }
}

fn reference(mod_id: &str) -> CatalogReference {
    CatalogReference {
        bucket: "deps".into(),
        game_id: "test-game".into(),
        mod_id: mod_id.into(),
    }
}

#[test]
fn override_second_install_replaces_first_bytes_and_keeps_first_package() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", b"first-bytes");
    let second_archive = fixture.archive("second", b"second-bytes");
    let first = fixture.install(&first_archive).expect("first");
    let second = fixture.install(&second_archive).expect("second");

    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"second-bytes");
    assert_ne!(first.id, second.id);
    assert_eq!(
        fixture.store.installed(&fixture.game).expect("list").len(),
        2
    );
    assert_eq!(fixture.winner().as_deref(), Some(second.id.as_str()));
}

#[test]
fn override_removes_witness_after_commit() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    fixture.install(&first).expect("first");
    fixture.install(&second).expect("second");

    let witnesses: Vec<_> = fs::read_dir(fixture.game.join("addons"))
        .expect("addons")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".modn-txn-"))
        .collect();
    assert!(witnesses.is_empty(), "{witnesses:?}");
}

#[test]
fn override_tampered_owned_file_is_refused_without_change() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    fixture.install(&first).expect("first");
    fs::write(fixture.path(), b"user-change").expect("tamper");

    let error = fixture.install(&second).expect_err("refuse");
    assert!(error.to_string().contains("CONFLICT"), "{error:#}");
    assert!(
        error.to_string().contains("owned file changed"),
        "{error:#}"
    );
    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"user-change");
    assert_eq!(
        fixture.store.installed(&fixture.game).expect("list").len(),
        1
    );
    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 1);
    assert!(fixture.store.pending().expect("pending").is_empty());
}

#[test]
fn override_unmanaged_destination_is_refused() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    fs::create_dir_all(fixture.game.join("addons")).expect("dirs");
    fs::write(fixture.path(), b"user-file").expect("user file");
    let _ = first;

    let error = fixture.install(&second).expect_err("refuse");
    assert!(error.to_string().contains("EXISTING_FILE"), "{error:#}");
    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"user-file");
    assert_eq!(fixture.count("SELECT COUNT(*) FROM managed_packages"), 0);
}

#[test]
fn override_symlink_destination_is_refused() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    fixture.install(&first).expect("first");
    let target = fixture.path();
    fs::remove_file(&target).expect("remove owned");
    let outside = fixture.game.join("bin/TestGame.exe");
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, &target).is_ok();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, &target).is_ok();
    if !linked {
        return;
    }
    let second = fixture.archive("second", b"second-bytes");
    let error = fixture.install(&second).expect_err("refuse");
    assert!(
        error.to_string().contains("LINKED_DESTINATION"),
        "{error:#}"
    );
    assert_eq!(fs::read(&outside).expect("bytes"), b"game-exe");
}

#[test]
fn override_claims_record_winner_and_shadowed_rows() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    let first = fixture.install(&first).expect("first");
    let second = fixture.install(&second).expect("second");

    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 2);
    assert_eq!(fixture.winner().as_deref(), Some(second.id.as_str()));
    let shadowed: String = fixture
        .store
        .connection
        .query_row(
            "SELECT package_id FROM file_claims WHERE package_id=?1",
            [&first.id],
            |row| row.get(0),
        )
        .expect("shadowed claim");
    assert_eq!(shadowed, first.id);
}

#[test]
fn override_managed_files_winner_row_has_profile_and_archive_entry() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    fixture.install(&first).expect("first");
    let second = fixture.install(&second).expect("second");

    let (package, profile, entry): (String, String, String) = fixture
        .store
        .connection
        .query_row(
            "SELECT package_id, profile_id, archive_entry FROM managed_files WHERE relative_path=?1",
            [SHARED],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("winner row");
    assert_eq!(package, second.id);
    assert_eq!(entry, "pack/shared.plugin");
    let active: String = fixture
        .store
        .connection
        .query_row(
            "SELECT id FROM profiles WHERE game_path=?1 AND active=1",
            [fixture.game.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .expect("active profile");
    assert_eq!(profile, active);
}

#[test]
fn override_three_packages_share_one_path() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    let third = fixture.archive("third", b"third-bytes");
    fixture.install(&first).expect("first");
    fixture.install(&second).expect("second");
    let third = fixture.install(&third).expect("third");

    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"third-bytes");
    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 3);
    assert_eq!(fixture.winner().as_deref(), Some(third.id.as_str()));
    assert_eq!(
        fixture.store.installed(&fixture.game).expect("list").len(),
        3
    );
}

#[test]
fn override_membership_priorities_append_in_install_order() {
    let mut fixture = Fixture::new();
    for name in ["first", "second", "third"] {
        let archive = fixture.archive(name, name.as_bytes());
        fixture.install(&archive).expect("install");
    }
    let mut statement = fixture
        .store
        .connection
        .prepare("SELECT priority FROM profile_packages ORDER BY priority")
        .expect("query");
    let priorities: Vec<i64> = statement
        .query_map([], |row| row.get(0))
        .expect("rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect");
    assert_eq!(priorities, vec![0, 1, 2]);
}

#[test]
fn override_uninstall_winner_with_shadowed_claim_is_refused() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    fixture.install(&first).expect("first");
    let second = fixture.install(&second).expect("second");
    let game = fixture.game.clone();

    let error = fixture
        .store
        .uninstall(&game, &second.id)
        .expect_err("refuse");
    assert!(error.to_string().contains(SHARED), "{error}");
    assert!(
        error.to_string().contains("lower-priority package first"),
        "{error}"
    );
    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"second-bytes");
}

#[test]
fn override_uninstall_shadowed_package_leaves_winner_bytes() {
    let mut fixture = Fixture::new();
    let first = fixture.archive("first", b"first-bytes");
    let second = fixture.archive("second", b"second-bytes");
    let first = fixture.install(&first).expect("first");
    let second = fixture.install(&second).expect("second");
    let game = fixture.game.clone();

    fixture
        .store
        .uninstall(&game, &first.id)
        .expect("uninstall");
    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"second-bytes");
    assert_eq!(fixture.winner().as_deref(), Some(second.id.as_str()));
    assert_eq!(fixture.count("SELECT COUNT(*) FROM file_claims"), 1);
    assert_eq!(fixture.store.installed(&game).expect("list").len(), 1);
}

#[test]
fn override_update_is_refused_for_a_shadowed_claim() {
    let mut fixture = Fixture::new();
    let first_archive = fixture.archive("first", b"first-bytes");
    let second_archive = fixture.archive("second", b"second-bytes");
    let update_archive = fixture.archive("update", b"update-bytes");
    let release = CatalogRelease::new(reference("first"), Vec::new());
    fixture
        .install_release(&first_archive, &release)
        .expect("first");
    fixture.install(&second_archive).expect("second");
    let old = fixture
        .store
        .installed(&fixture.game)
        .expect("list")
        .into_iter()
        .find(|package| package.catalog.is_some())
        .expect("catalog package");
    let game = fixture.game.clone();
    let plan = fixture.plan(&update_archive, SHARED);

    let error = fixture
        .store
        .update_catalog_release_planned(
            &game,
            &old.id,
            &update_archive,
            &release,
            NativeInjectorPolicy::Refuse,
            &[],
            |_archive, _package, _store| Ok(plan),
        )
        .expect_err("refuse");
    assert!(error.to_string().contains(SHARED), "{error}");
    assert_eq!(fs::read(fixture.path()).expect("bytes"), b"second-bytes");
}

#[test]
fn override_non_overlapping_install_writes_a_winner_claim() {
    let mut fixture = Fixture::new();
    let archive = fixture.archive("plain", b"plain-bytes");
    let plan = fixture.plan(&archive, "addons/other.plugin");
    let game = fixture.game.clone();
    let package = fixture
        .store
        .install_operations_zip_with_permissions(&game, &archive, &plan, &[])
        .expect("install");

    let (claim, entry): (String, String) = fixture
        .store
        .connection
        .query_row(
            "SELECT package_id, archive_entry FROM file_claims WHERE relative_path='addons/other.plugin'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("claim");
    assert_eq!(claim, package.id);
    assert_eq!(entry, "pack/shared.plugin");
}
