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
                owned_directory: None,
            })
            .collect(),
        blockers: Vec::new(),
        skipped_generated_files: Vec::new(),
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
        .insert_journal(
            "pending-test",
            "install",
            &game,
            "pending-test",
            None,
            &[],
            &[],
        )
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

#[test]
fn nested_root_cannot_take_ownership_of_missing_managed_files() {
    let (_dir, mut store, game, archive) = fixture();
    let original = plan(&game, &archive);
    let owned = store
        .install_operations_zip(&game, &archive, &original)
        .unwrap();
    let nested = game.join("data");
    for file in &owned.files {
        fs::remove_file(game.join(&file.destination)).unwrap();
    }
    let mut request = plan(&nested, &archive);
    for operation in &mut request.operations {
        operation.destination = operation.destination.strip_prefix("data/").unwrap().into();
    }
    assert!(store
        .install_operations_zip(&nested, &archive, &request)
        .is_err());
    assert!(store.pending().unwrap().is_empty());
    assert!(store.installed(&nested).unwrap().is_empty());
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    assert!(!game.join("data/mods/a.bin").exists());
}

#[test]
fn parent_root_cannot_take_ownership_of_nested_managed_installation() {
    let (_dir, mut store, game, archive) = fixture();
    let nested = game.join("data");
    fs::create_dir(&nested).unwrap();
    let mut nested_plan = plan(&nested, &archive);
    for operation in &mut nested_plan.operations {
        operation.destination = operation.destination.strip_prefix("data/").unwrap().into();
    }
    let owned = store
        .install_operations_zip(&nested, &archive, &nested_plan)
        .unwrap();
    for file in &owned.files {
        fs::remove_file(nested.join(&file.destination)).unwrap();
    }
    let parent_plan = plan(&game, &archive);
    assert!(store
        .install_operations_zip(&game, &archive, &parent_plan)
        .is_err());
    assert!(store.pending().unwrap().is_empty());
    assert_eq!(store.installed(&nested).unwrap().len(), 1);
    assert!(store.installed(&game).unwrap().is_empty());
}

#[cfg(windows)]
#[test]
fn short_directory_alias_cannot_reclaim_missing_owned_files() {
    let (_dir, mut store, game, archive) = fixture();
    let long = game.join("r6/scripts/LongDirectoryName");
    fs::create_dir_all(&long).unwrap();
    let command = format!("for %I in ({}) do @echo %~sI", long.display());
    let response = std::process::Command::new("cmd")
        .args(["/C", &command])
        .output()
        .unwrap();
    assert!(response.status.success());
    let short_path = String::from_utf8_lossy(&response.stdout).trim().to_owned();
    let short = Path::new(&short_path)
        .file_name()
        .unwrap()
        .to_string_lossy();
    if short.eq_ignore_ascii_case("LongDirectoryName") {
        eprintln!(
            "NTFS 8.3 aliases unavailable for new test directories; skipping integration path"
        );
        return;
    }
    assert!(
        Path::new(&short_path).exists(),
        "short path: {short_path:?}"
    );
    assert_eq!(
        Path::new(&short_path).canonicalize().unwrap(),
        long.canonicalize().unwrap()
    );

    let mut first = plan(&game, &archive);
    for operation in &mut first.operations {
        let name = operation.destination.rsplit('/').next().unwrap();
        operation.destination = format!("r6/scripts/LongDirectoryName/{name}");
    }
    let owned = store
        .install_operations_zip(&game, &archive, &first)
        .unwrap();
    for file in &owned.files {
        fs::remove_file(game.join(&file.destination)).unwrap();
    }

    let mut alias = plan(&game, &archive);
    for operation in &mut alias.operations {
        let name = operation.destination.rsplit('/').next().unwrap();
        operation.destination = format!("r6/scripts/{short}/{name}");
    }
    let error = store
        .install_operations_zip(&game, &archive, &alias)
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("ALIASED_DESTINATION"),
        "{error:#}"
    );
    assert!(store.pending().unwrap().is_empty());
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    assert!(store.installed(&game).unwrap()[0].id == owned.id);
    for file in &owned.files {
        assert!(!game.join(&file.destination).exists());
    }
}

#[cfg(windows)]
#[test]
fn existing_unicode_case_variant_directory_remains_installable() {
    let (_dir, mut store, game, archive) = fixture();
    let actual = game.join("r6/scripts/\u{00C9}cole");
    fs::create_dir_all(&actual).unwrap();
    let alternate = game.join("r6/scripts/\u{00E9}cole");
    assert_eq!(
        actual.canonicalize().unwrap(),
        alternate.canonicalize().unwrap()
    );

    let mut proposal = plan(&game, &archive);
    for operation in &mut proposal.operations {
        let name = operation.destination.rsplit('/').next().unwrap();
        operation.destination = format!("r6/scripts/\u{00E9}cole/{name}");
    }
    let owned = store
        .install_operations_zip(&game, &archive, &proposal)
        .unwrap();
    assert_eq!(fs::read(actual.join("a.bin")).unwrap(), b"one");
    assert_eq!(fs::read(alternate.join("b.bin")).unwrap(), b"two");
    store.uninstall(&game, &owned.id).unwrap();
    assert!(!actual.join("a.bin").exists());
    assert!(!actual.join("b.bin").exists());
}

#[test]
fn exclusive_directory_owns_runtime_contents_without_file_hash_checks() {
    let (_tmp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut requested = plan(&game, &archive);
    for op in &mut requested.operations {
        op.owned_directory = Some("data/mods".into());
    }
    let owned = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    assert_eq!(owned.directories, ["data/mods"]);
    assert_eq!(
        store.installed(&game).unwrap()[0].directories,
        ["data/mods"]
    );
    fs::write(game.join("data/mods/a.bin"), b"changed at runtime").unwrap();
    fs::write(game.join("data/mods/runtime.log"), b"new user log").unwrap();
    store.uninstall(&game, &owned.id).unwrap();
    assert!(!game.join("data/mods").exists());
    assert!(game.join("data").is_dir());
    assert!(store.installed(&game).unwrap().is_empty());
    assert_eq!(store.recover().unwrap(), 0);
}

#[test]
fn mixed_directory_and_shared_file_preserves_modified_shared_file() {
    let (_tmp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut requested = plan(&game, &archive);
    requested.operations[0].owned_directory = Some("data/mods".into());
    requested.operations[1].destination = "data/shared.xml".into();
    let owned = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    assert_eq!(owned.directories, ["data/mods"]);
    fs::write(game.join("data/mods/created.db"), b"runtime").unwrap();
    fs::write(game.join("data/shared.xml"), b"user changed this").unwrap();
    assert!(store.uninstall(&game, &owned.id).is_err());
    assert_eq!(
        fs::read(game.join("data/shared.xml")).unwrap(),
        b"user changed this"
    );
    assert!(game.join("data/mods/created.db").is_file());
    fs::write(game.join("data/shared.xml"), b"two").unwrap();
    store.uninstall(&game, &owned.id).unwrap();
    assert!(!game.join("data/mods").exists());
    assert!(!game.join("data/shared.xml").exists());
    assert!(game.join("data").exists());
}

#[test]
fn refuses_claims_over_preexisting_or_reserved_content() {
    let (_tmp, mut store, game, archive) = fixture();
    fs::create_dir_all(game.join("data/mods")).unwrap();
    fs::write(game.join("data/mods/manual.log"), b"manual").unwrap();
    let mut requested = plan(&game, &archive);
    for op in &mut requested.operations {
        op.owned_directory = Some("data/mods".into());
    }
    assert!(store
        .install_operations_zip(&game, &archive, &requested)
        .is_err());
    assert_eq!(
        fs::read(game.join("data/mods/manual.log")).unwrap(),
        b"manual"
    );
    assert!(store.installed(&game).unwrap().is_empty());
    fs::remove_dir_all(game.join("data/mods")).unwrap();
    let owned = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    // Even a new distinct filename cannot enter a directory claimed by another package.
    let mut second = plan(&game, &archive);
    second.operations[0].destination = "data/mods/new.bin".into();
    second.operations.remove(1);
    assert!(store
        .install_operations_zip(&game, &archive, &second)
        .is_err());
    assert_eq!(store.installed(&game).unwrap()[0].id, owned.id);
}

#[test]
fn pending_directory_removal_is_restored_and_committed_removal_is_cleaned() {
    let (_tmp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut requested = plan(&game, &archive);
    for op in &mut requested.operations {
        op.owned_directory = Some("data/mods".into());
    }
    let owned = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    fs::write(game.join("data/mods/generated.log"), b"runtime").unwrap();
    let id = "txn-dir-interrupted";
    store
        .insert_journal(
            id,
            "uninstall",
            &game,
            &owned.id,
            None,
            &[],
            &owned.directories,
        )
        .unwrap();
    super::directory::quarantine(&game, "data/mods", id, 0).unwrap();
    assert!(!game.join("data/mods").exists());
    assert_eq!(store.recover().unwrap(), 1);
    assert_eq!(
        fs::read(game.join("data/mods/generated.log")).unwrap(),
        b"runtime"
    );
    assert_eq!(store.installed(&game).unwrap().len(), 1);
    let id = "txn-dir-committed";
    store
        .insert_journal(
            id,
            "uninstall",
            &game,
            &owned.id,
            None,
            &[],
            &owned.directories,
        )
        .unwrap();
    super::directory::quarantine(&game, "data/mods", id, 0).unwrap();
    store.commit_uninstall(id, &owned.id).unwrap();
    assert_eq!(store.recover().unwrap(), 1);
    assert!(!game.join("data/mods").exists());
    assert!(store.installed(&game).unwrap().is_empty());
}
