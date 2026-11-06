use super::journal::{target_path, temp_path, JournalFile};
use crate::state::Store;
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;

fn install_zip(
    store: &mut Store,
    game: &Path,
    archive: &Path,
) -> anyhow::Result<super::ManagedPackage> {
    let proposal = crate::planning::preview_install_zip("cyberpunk-2077", game, archive)?;
    store.install_operations_zip(game, archive, &proposal)
}

fn fixture() -> (tempfile::TempDir, Store, PathBuf) {
    let root = tempdir().unwrap();
    let game = root.path().join("game");
    fs::create_dir_all(game.join("bin/x64")).unwrap();
    fs::write(game.join("bin/x64/Cyberpunk2077.exe"), b"game").unwrap();
    let store = Store::open(&root.path().join("state/state.sqlite")).unwrap();
    (root, store, game)
}

fn archive(root: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
    let path = root.join("fixture.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    for (name, contents) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
    path
}

fn prepared_file(dest: &str, data: &[u8]) -> JournalFile {
    let mut hasher = Sha256::new();
    hasher.update(data);
    JournalFile {
        root: crate::state::RootKind::Game,
        ordinal: 0,
        destination: dest.into(),
        sha256: format!("{:x}", hasher.finalize()),
        bytes: data.len() as u64,
    }
}

#[test]
fn install_list_and_uninstall_preserve_unmanaged_files() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("Wrapper/archive/pc/mod/HD Reworked.archive", b"new mod"),
                ("Wrapper/README.md", b"ignored"),
            ],
        );
        fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
        let unmanaged = game.join("archive/pc/mod/unmanaged.archive");
        fs::write(&unmanaged, b"keep").unwrap();
        let installed = install_zip(&mut store, &game, &path).unwrap();
        assert_eq!(installed.files.len(), 1);
        assert_eq!(
            fs::read(game.join(&installed.files[0].destination)).unwrap(),
            b"new mod"
        );
        assert_eq!(store.installed(&game).unwrap().len(), 1);
        store.uninstall(&game, &installed.id).unwrap();
        assert!(store.installed(&game).unwrap().is_empty());
        assert!(!game.join(&installed.files[0].destination).exists());
        assert_eq!(fs::read(unmanaged).unwrap(), b"keep");
    });
}

#[test]
fn existing_file_blocks_install_without_changing_it() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let path = archive(dir.path(), &[("archive/pc/mod/x.archive", b"replacement")]);
        fs::create_dir_all(game.join("archive/pc/mod")).unwrap();
        let dest = game.join("archive/pc/mod/x.archive");
        fs::write(&dest, b"unmanaged").unwrap();
        assert!(install_zip(&mut store, &game, &path).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"unmanaged");
        assert!(store.installed(&game).unwrap().is_empty());
    });
}

#[test]
fn modified_managed_content_cannot_be_uninstalled() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let path = archive(dir.path(), &[("archive/pc/mod/x.archive", b"original")]);
        let package = install_zip(&mut store, &game, &path).unwrap();
        let dest = game.join(&package.files[0].destination);
        fs::write(&dest, b"user edits").unwrap();
        assert!(store.uninstall(&game, &package.id).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"user edits");
        assert_eq!(store.installed(&game).unwrap().len(), 1);
    });
}

#[test]
fn injected_partial_install_rolls_back_all_owned_files() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("archive/pc/mod/one.archive", b"first"),
                ("archive/pc/mod/two.archive", b"second"),
            ],
        );
        assert!(store
            .install_operations_inner(
                &game,
                &path,
                &crate::planning::preview_install_zip("cyberpunk-2077", &game, &path).unwrap(),
                Some(1),
            )
            .is_err());
        assert!(!game.join("archive/pc/mod/one.archive").exists());
        assert!(!game.join("archive/pc/mod/two.archive").exists());
        assert!(store.installed(&game).unwrap().is_empty());
        assert_eq!(store.recover().unwrap(), 0);
        assert!(install_zip(&mut store, &game, &path).is_ok());
    });
}

#[test]
fn recovering_crashed_install_removes_only_proven_hardlinks() {
    crate::test_support::seeded(|| {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-crash";
        store
            .insert_journal(
                id,
                "install",
                &game,
                id,
                Some("archive"),
                std::slice::from_ref(&f),
                &[],
            )
            .unwrap();
        let dest = test_target(&game, &f.destination);
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::write(&temp, b"owned").unwrap();
        fs::hard_link(&temp, &dest).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!dest.exists());
        assert!(!temp.exists());
        assert_eq!(store.recover().unwrap(), 0);
    });
}

#[test]
fn recovering_crashed_uninstall_restores_original_file() {
    crate::test_support::seeded(|| {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-uninstall";
        let dest = test_target(&game, &f.destination);
        fs::write(&dest, b"owned").unwrap();
        store
            .insert_journal(
                id,
                "uninstall",
                &game,
                id,
                None,
                std::slice::from_ref(&f),
                &[],
            )
            .unwrap();
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::hard_link(&dest, &temp).unwrap();
        fs::remove_file(&dest).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert_eq!(fs::read(&dest).unwrap(), b"owned");
        assert!(!temp.exists());
    });
}

#[test]
fn changed_witness_blocks_recovery_and_preserves_files() {
    crate::test_support::seeded(|| {
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/crashed.archive", b"owned");
        let id = "txn-test-tamper";
        store
            .insert_journal(
                id,
                "install",
                &game,
                id,
                None,
                std::slice::from_ref(&f),
                &[],
            )
            .unwrap();
        let dest = test_target(&game, &f.destination);
        let temp = temp_path(&dest, id, 0).unwrap();
        fs::write(&temp, b"owned").unwrap();
        fs::hard_link(&temp, &dest).unwrap();
        fs::write(&dest, b"changed").unwrap();
        assert!(store.recover().is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"changed");
        assert!(install_zip(
            &mut store,
            &game,
            &archive(_dir.path(), &[("archive/pc/mod/other.archive", b"x")])
        )
        .is_err());
    });
}

/// Leaves a pending install journal with a linked witness, written by a Modn
/// that predates link records, and reopens the state with the current Modn.
fn older_linked_install(
    root: &Path,
    mut store: Store,
    game: &Path,
    id: &str,
) -> (Store, PathBuf, PathBuf) {
    let f = prepared_file("archive/pc/mod/older.archive", b"owned");
    store
        .insert_journal(
            id,
            "install",
            game,
            id,
            Some("archive"),
            std::slice::from_ref(&f),
            &[],
        )
        .unwrap();
    store
        .connection
        .execute_batch("DROP TABLE pending_links; DROP TABLE link_recorded_journals;")
        .unwrap();
    drop(store);
    let dest = test_target(game, &f.destination);
    let temp = temp_path(&dest, id, 0).unwrap();
    fs::write(&temp, b"owned").unwrap();
    fs::hard_link(&temp, &dest).unwrap();
    let store = Store::open(&root.join("state/state.sqlite")).unwrap();
    (store, dest, temp)
}

#[test]
fn recovery_keeps_a_changed_sole_witness_from_an_older_journal() {
    crate::test_support::seeded(|| {
        let (dir, store, game) = fixture();
        let (mut store, dest, temp) = older_linked_install(dir.path(), store, &game, "txn-older-a");
        fs::write(&dest, b"unique user data").unwrap();
        fs::remove_file(&dest).unwrap();

        let error = store.recover().unwrap_err();
        assert!(
            error.to_string().contains("changed or incomplete witness"),
            "{error:#}"
        );
        assert_eq!(fs::read(&temp).unwrap(), b"unique user data");
        assert_eq!(store.pending().unwrap().len(), 1);
    });
}

#[test]
fn recovery_rolls_back_a_linked_destination_from_an_older_journal() {
    crate::test_support::seeded(|| {
        let (dir, store, game) = fixture();
        let (mut store, dest, temp) = older_linked_install(dir.path(), store, &game, "txn-older-b");

        assert_eq!(store.recover().unwrap(), 1);
        assert!(!dest.exists());
        assert!(!temp.exists());
        assert!(store.pending().unwrap().is_empty());
    });
}

#[test]
fn recovering_crashed_extraction_removes_staging_without_game_writes() {
    crate::test_support::seeded(|| {
        let (_dir, mut store, game) = fixture();
        let id = "txn-crashed-extraction";
        let stage = store.state_root.join(id);
        fs::create_dir_all(&stage).unwrap();
        store.begin_staging(id, &game, "sha", None).unwrap();
        fs::write(stage.join("0.bin"), vec![17u8; 4096]).unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!stage.exists());
        assert!(store.pending().unwrap().is_empty());
        assert!(!game.join("archive").exists());
    });
}

#[cfg(windows)]
#[test]
fn locked_staging_cleanup_keeps_journal_for_retry() {
    crate::test_support::seeded(|| {
        use std::os::windows::fs::OpenOptionsExt;
        let (_dir, mut store, game) = fixture();
        let f = prepared_file("archive/pc/mod/never-created.archive", b"not installed");
        let id = "txn-locked-staging";
        store
            .insert_journal(
                id,
                "install",
                &game,
                id,
                None,
                std::slice::from_ref(&f),
                &[],
            )
            .unwrap();
        let stage = store.state_root.join(id);
        fs::create_dir_all(&stage).unwrap();
        let file = stage.join("locked.bin");
        fs::write(&file, b"staged").unwrap();
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)
            .unwrap();
        assert!(store.recover().is_err());
        assert_eq!(store.pending().unwrap().len(), 1);
        drop(handle);
        assert_eq!(store.recover().unwrap(), 1);
        assert!(!stage.exists());
        assert!(store.pending().unwrap().is_empty());
    });
}

#[test]
fn recovery_never_deletes_preexisting_empty_directories() {
    crate::test_support::seeded(|| {
        let (_dir, mut store, game) = fixture();
        let directory = game.join("archive/pc/mod");
        fs::create_dir_all(&directory).unwrap();
        let file = prepared_file("archive/pc/mod/not-written.archive", b"never written");
        store
            .insert_journal(
                "txn-empty-dir",
                "install",
                &game,
                "txn-empty-dir",
                None,
                std::slice::from_ref(&file),
                &[],
            )
            .unwrap();
        assert_eq!(store.recover().unwrap(), 1);
        assert!(directory.is_dir());
    });
}

#[test]
fn rejects_non_zip_or_mixed_framework_package() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let path = archive(
            dir.path(),
            &[
                ("archive/pc/mod/x.archive", b"new"),
                ("r6/scripts/script.reds", b"script"),
            ],
        );
        assert!(install_zip(&mut store, &game, &path).is_err());
        assert!(!game.join("archive").exists());
        let other = dir.path().join("fake.7z");
        fs::write(&other, b"wrong format").unwrap();
        assert!(install_zip(&mut store, &game, &other).is_err());
    });
}

#[test]
fn cet_and_red4ext_multifile_injected_failure_rolls_back_and_recovers() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let cet = "bin/x64/plugins/cyber_engine_tweaks/mods/Fixture/init.lua";
        let red = "red4ext/plugins/Fixture/Fixture.dll";
        let path = archive(dir.path(), &[(cet, b"lua"), (red, b"plugin")]);
        let proposal = crate::planning::preview_install_zip("cyberpunk-2077", &game, &path)
            .expect("Lua mixed CET/RED4ext plan");
        assert_eq!(proposal.status, crate::planning::PlanStatus::Reviewable);
        assert_eq!(proposal.operations.len(), 2);
        assert!(store
            .install_operations_inner(&game, &path, &proposal, Some(1))
            .is_err());
        assert!(!game.join(cet).exists());
        assert!(!game.join(red).exists());
        assert!(store.pending().expect("journal state").is_empty());
        assert!(store.installed(&game).expect("ownership state").is_empty());
        assert_eq!(store.recover().expect("idle recovery"), 0);
        let package = store
            .install_operations_zip(&game, &path, &proposal)
            .expect("normal mixed install");
        assert_eq!(package.files.len(), 2);
        store.uninstall(&game, &package.id).expect("safe uninstall");
        assert!(!game.join(cet).exists());
        assert!(!game.join(red).exists());
    });
}

#[test]
fn redscript_archivexl_mixed_failure_preserves_manual_files_and_recovers() {
    crate::test_support::seeded(|| {
        let (dir, mut store, game) = fixture();
        let compiler = "engine/tools/scc.exe";
        let xl_plugin = "red4ext/plugins/ArchiveXL/ArchiveXL.dll";
        for dependency in [compiler, xl_plugin] {
            let location = game.join(dependency);
            fs::create_dir_all(location.parent().expect("framework parent")).expect("parent");
            fs::write(location, b"manual framework").expect("framework");
        }
        let script = "r6/scripts/Fixture/main.reds";
        let xl = "archive/pc/mod/Fixture.archive.xl";
        let legacy = "archive/pc/mod/Fixture.archive";
        let archive = archive(
            dir.path(),
            &[(script, b"script"), (xl, b"xl"), (legacy, b"archive")],
        );
        let proposal = crate::planning::preview_install_zip("cyberpunk-2077", &game, &archive)
            .expect("Lua mixed redscript and ArchiveXL plan");
        assert_eq!(proposal.status, crate::planning::PlanStatus::Reviewable);
        assert_eq!(proposal.operations.len(), 3);
        assert!(store
            .install_operations_inner(&game, &archive, &proposal, Some(1))
            .is_err());
        for name in [script, xl, legacy] {
            assert!(!game.join(name).exists(), "rolled-back file {name}");
        }
        for dependency in [compiler, xl_plugin] {
            assert_eq!(
                fs::read(game.join(dependency)).expect("manual dependency"),
                b"manual framework"
            );
        }
        assert!(store.pending().expect("journal state").is_empty());
        assert!(store.installed(&game).expect("ownership").is_empty());
        assert_eq!(store.recover().expect("idle recovery"), 0);
        let managed = store
            .install_operations_zip(&game, &archive, &proposal)
            .expect("install");
        assert_eq!(managed.files.len(), 3);
        store.uninstall(&game, &managed.id).expect("uninstall");
        for name in [script, xl, legacy] {
            assert!(!game.join(name).exists());
        }
    });
}

fn test_target(game: &Path, relative: &str) -> PathBuf {
    fs::create_dir_all(game.join(relative).parent().unwrap()).unwrap();
    target_path(game, relative).unwrap()
}
