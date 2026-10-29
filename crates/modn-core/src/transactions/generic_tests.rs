use crate::archive::inspect;
use crate::planning::{InstallPlan, PlanIssue, PlannedFile};
use crate::state::Store;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::{tempdir, TempDir};
use zip::write::SimpleFileOptions;

fn fixture() -> (TempDir, Store, PathBuf, PathBuf) {
    let dir = tempdir().unwrap();
    let game = dir.path().join("other-game");
    fs::create_dir(&game).unwrap();
    let store = Store::open(&dir.path().join("state/state.sqlite")).unwrap();
    let archive = dir.path().join("package.zip");
    make_zip(&archive, &[("pack/a.bin", b"one"), ("pack/b.bin", b"two")]);
    (dir, store, game, archive)
}
fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}
fn plan(game: &Path, archive: &Path) -> InstallPlan {
    let checked = inspect(archive).unwrap();
    InstallPlan {
        archive_sha256: checked.sha256,
        game_installation: game.canonicalize().unwrap(),
        status: "reviewable".into(),
        metadata_only: true,
        operations: checked
            .files
            .into_iter()
            .map(|entry| PlannedFile {
                destination: format!("data/mods/{}", entry.entry.rsplit('/').next().unwrap()),
                source: entry.entry,
                bytes: entry.bytes,
                kind: "file-copy".into(),
            })
            .collect(),
        blockers: Vec::new(),
    }
}
#[test]
fn non_cyberpunk_root_install_list_uninstall_and_recover() {
    let (_dir, mut store, game, archive) = fixture();
    let package = store
        .install_operations_zip(&game, &archive, &plan(&game, &archive))
        .unwrap();
    assert_eq!(package.files.len(), 2);
    assert_eq!(fs::read(game.join("data/mods/a.bin")).unwrap(), b"one");
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    store.uninstall(&game, &package.id).unwrap();
    assert_eq!(store.recover().unwrap(), 0);
    assert!(!game.join("data/mods/a.bin").exists());
}
#[test]
fn independently_rejects_forged_plans_without_writes_or_journals() {
    let (dir, mut store, game, archive) = fixture();
    for choice in 0..11 {
        let mut proposal = plan(&game, &archive);
        match choice {
            0 => proposal.status = "blocked".into(),
            1 => proposal.blockers.push(PlanIssue {
                code: "BLOCK".into(),
                message: "blocked".into(),
            }),
            2 => proposal.archive_sha256 = "stale".into(),
            3 => proposal.game_installation = dir.path().to_path_buf(),
            4 => proposal.operations[0].source = "missing/file.bin".into(),
            5 => proposal.operations[0].bytes += 1,
            6 => proposal.operations[0].destination = "../escape/a".into(),
            7 => proposal.operations[0].destination = "data/mods/install.exe".into(),
            8 => proposal.operations[1].destination = proposal.operations[0].destination.clone(),
            9 => proposal.operations[0].kind = "".into(),
            10 => proposal.operations[0].destination = "data/mods".into(),
            _ => unreachable!(),
        }
        assert!(
            store
                .install_operations_zip(&game, &archive, &proposal)
                .is_err(),
            "{choice}"
        );
        assert!(!game.join("data").exists(), "{choice}");
        assert!(store.pending().unwrap().is_empty());
    }
}
#[test]
fn rejects_live_conflicts_archive_changes_and_executable_sources() {
    let (_dir, mut store, game, archive) = fixture();
    let proposal = plan(&game, &archive);
    fs::create_dir_all(game.join("data/mods")).unwrap();
    fs::write(game.join("data/mods/a.bin"), b"unmanaged").unwrap();
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_err());
    assert_eq!(
        fs::read(game.join("data/mods/a.bin")).unwrap(),
        b"unmanaged"
    );
    fs::remove_file(game.join("data/mods/a.bin")).unwrap();
    make_zip(
        &archive,
        &[("pack/a.bin", b"changed"), ("pack/b.bin", b"two")],
    );
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_err());
    make_zip(
        &archive,
        &[("pack/tool.exe", b"exe"), ("pack/b.bin", b"two")],
    );
    let mut suspicious = plan(&game, &archive);
    suspicious.operations.remove(0);
    assert!(store
        .install_operations_zip(&game, &archive, &suspicious)
        .is_err());
    assert!(store.pending().unwrap().is_empty());
}
#[test]
fn injected_failure_rolls_back_generic_operations() {
    let (_dir, mut store, game, archive) = fixture();
    let proposal = plan(&game, &archive);
    assert!(store
        .install_operations_inner(&game, &archive, &proposal, Some(1))
        .is_err());
    assert!(!game.join("data/mods/a.bin").exists());
    assert!(!game.join("data/mods/b.bin").exists());
    assert!(store.installed(&game).unwrap().is_empty());
    assert_eq!(store.recover().unwrap(), 0);
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_ok());
}
#[cfg(windows)]
#[test]
fn junction_path_does_not_escape_generic_root() {
    let (dir, mut store, game, archive) = fixture();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let link = game.join("data");
    let result = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(store
        .install_operations_zip(&game, &archive, &plan(&game, &archive))
        .is_err());
    assert!(!outside.join("mods/a.bin").exists());
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn missing_managed_targets_remain_reserved_and_block_reinstallation() {
    let (_dir, mut store, game, archive) = fixture();
    let proposal = plan(&game, &archive);
    let package = store
        .install_operations_zip(&game, &archive, &proposal)
        .unwrap();
    for owned in &package.files {
        fs::remove_file(game.join(&owned.destination)).unwrap();
    }
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_err());
    assert!(store.pending().unwrap().is_empty());
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    assert!(!game.join("data/mods/a.bin").exists());
}

#[test]
fn pending_journal_blocks_new_generic_transaction_until_recovery() {
    let (_dir, mut store, game, archive) = fixture();
    store
        .insert_journal("pending-test", "install", &game, "pending-test", None, &[])
        .unwrap();
    assert!(store
        .install_operations_zip(&game, &archive, &plan(&game, &archive))
        .is_err());
    assert_eq!(store.pending().unwrap().len(), 1);
    assert_eq!(store.recover().unwrap(), 1);
    assert!(store
        .install_operations_zip(&game, &archive, &plan(&game, &archive))
        .is_ok());
}

#[test]
fn generic_executor_refuses_jar_instead_of_zip() {
    let (dir, mut store, game, archive) = fixture();
    let jar = dir.path().join("package.jar");
    fs::copy(&archive, &jar).unwrap();
    assert!(store
        .install_operations_zip(&game, &jar, &plan(&game, &jar))
        .is_err());
    assert!(store.installed(&game).unwrap().is_empty());
}
