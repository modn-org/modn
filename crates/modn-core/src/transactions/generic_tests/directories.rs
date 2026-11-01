//! Exclusive-directory ownership, quarantine and recovery for generic roots.
use std::fs;

use super::{fixture, plan};

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
    crate::transactions::directory::quarantine(
        &game,
        "data/mods",
        id,
        0,
        &store
            .managed_directory_identity(&owned.id, "data/mods")
            .unwrap(),
    )
    .unwrap();
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
    crate::transactions::directory::quarantine(
        &game,
        "data/mods",
        id,
        0,
        &store
            .managed_directory_identity(&owned.id, "data/mods")
            .unwrap(),
    )
    .unwrap();
    store.commit_uninstall(id, &owned.id).unwrap();
    assert_eq!(store.recover().unwrap(), 1);
    assert!(!game.join("data/mods").exists());
    assert!(store.installed(&game).unwrap().is_empty());
}

#[test]
fn injected_exclusive_install_failure_removes_witness_and_nested_folder() {
    let (_temp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut requested = plan(&game, &archive);
    for op in &mut requested.operations {
        op.destination = format!("data/mods/deep/{}", op.source.rsplit('/').next().unwrap());
        op.owned_directory = Some("data/mods".into());
    }
    assert!(store
        .install_operations_inner(&game, &archive, &requested, Some(1))
        .is_err());
    assert!(!game.join("data/mods").exists());
    assert_eq!(store.recover().unwrap(), 0);
    assert!(store.pending().unwrap().is_empty());
    let owned = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    assert_eq!(owned.directories, ["data/mods"]);
    store.uninstall(&game, &owned.id).unwrap();
}

#[test]
fn substituted_quarantine_directory_blocks_committed_recovery() {
    let (_temp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut requested = plan(&game, &archive);
    for op in &mut requested.operations {
        op.owned_directory = Some("data/mods".into());
    }
    let package = store
        .install_operations_zip(&game, &archive, &requested)
        .unwrap();
    let id = "txn-swap-dir";
    store
        .insert_journal(
            id,
            "uninstall",
            &game,
            &package.id,
            None,
            &[],
            &package.directories,
        )
        .unwrap();
    crate::transactions::directory::quarantine(
        &game,
        "data/mods",
        id,
        0,
        &store
            .managed_directory_identity(&package.id, "data/mods")
            .unwrap(),
    )
    .unwrap();
    let staged =
        crate::transactions::directory::quarantine_path(&game, "data/mods", id, 0).unwrap();
    let saved = game.join("data/saved-original");
    fs::rename(&staged, &saved).unwrap();
    fs::create_dir(&staged).unwrap();
    // A forged directory can hard-link files from the original: file identity
    // must not authorize deletion of the different containing directory.
    fs::write(saved.join("original-marker"), b"valid marker bytes").unwrap();
    fs::hard_link(
        saved.join("original-marker"),
        staged.join(".modn-ownership"),
    )
    .unwrap();
    fs::write(staged.join("unmanaged.txt"), b"not modn's directory").unwrap();
    store.commit_uninstall(id, &package.id).unwrap();
    assert!(store.recover().is_err());
    assert_eq!(
        fs::read(staged.join("unmanaged.txt")).unwrap(),
        b"not modn's directory"
    );
    assert_eq!(store.pending().unwrap().len(), 1);
    fs::remove_dir_all(&staged).unwrap();
    fs::rename(&saved, &staged).unwrap();
    assert_eq!(store.recover().unwrap(), 1);
    assert!(!staged.exists());
}

#[cfg(windows)]
#[test]
fn mixed_case_alias_cannot_escape_exclusive_directory_validation() {
    let (_temp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut proposal = plan(&game, &archive);
    proposal.operations[0].destination = "data/Mods/a.bin".into();
    proposal.operations[0].owned_directory = Some("data/Mods".into());
    proposal.operations[1].destination = "data/mods/b.bin".into();
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_err());
    assert!(!game.join("data/Mods").exists());
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn committed_directory_cleanup_resumes_after_partial_removal() {
    let (_temp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut proposal = plan(&game, &archive);
    for op in &mut proposal.operations {
        op.owned_directory = Some("data/mods".into());
    }
    let installed = store
        .install_operations_zip(&game, &archive, &proposal)
        .unwrap();
    let id = "txn-partial-dir-cleanup";
    store
        .insert_journal(
            id,
            "uninstall",
            &game,
            &installed.id,
            None,
            &[],
            &installed.directories,
        )
        .unwrap();
    let witness = store
        .managed_directory_identity(&installed.id, "data/mods")
        .unwrap();
    crate::transactions::directory::quarantine(&game, "data/mods", id, 0, &witness).unwrap();
    store.commit_uninstall(id, &installed.id).unwrap();
    let staged =
        crate::transactions::directory::quarantine_path(&game, "data/mods", id, 0).unwrap();
    fs::remove_file(staged.join("a.bin")).unwrap();
    fs::write(staged.join("runtime-extra"), b"generated").unwrap();
    assert_eq!(store.recover().unwrap(), 1);
    assert!(!staged.exists());
    assert!(store.pending().unwrap().is_empty());
}

#[test]
fn interrupted_directory_preparation_only_removes_empty_unproven_staging() {
    let (_temp, mut store, game, _archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let id = "txn-unproven-stage";
    let root = "data/mods";
    store
        .insert_journal(id, "install", &game, id, None, &[], &[root.into()])
        .unwrap();
    let staged = crate::transactions::directory::quarantine_path(&game, root, id, 0).unwrap();
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join("unmanaged.txt"), b"external").unwrap();
    assert!(store.recover().is_err());
    assert_eq!(fs::read(staged.join("unmanaged.txt")).unwrap(), b"external");
    assert_eq!(store.pending().unwrap().len(), 1);
    fs::remove_file(staged.join("unmanaged.txt")).unwrap();
    assert_eq!(store.recover().unwrap(), 1);
    assert!(!staged.exists());
    assert!(!game.join(root).exists());
}

#[cfg(windows)]
#[test]
fn greek_sigma_alias_cannot_mix_directory_and_file_ownership() {
    let (_temp, mut store, game, archive) = fixture();
    fs::create_dir(game.join("data")).unwrap();
    let mut proposal = plan(&game, &archive);
    proposal.operations[0].destination = "data/\u{039F}\u{03A3}/a.bin".into();
    proposal.operations[0].owned_directory = Some("data/\u{039F}\u{03A3}".into());
    proposal.operations[1].destination = "data/\u{03BF}\u{03C3}/b.bin".into();
    assert!(store
        .install_operations_zip(&game, &archive, &proposal)
        .is_err());
    assert!(!game.join("data/\u{039F}\u{03A3}").exists());
    assert!(store.pending().unwrap().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn linux_birth_identity_rejects_reused_inode_generation() {
    let (_temp, _store, game, _archive) = fixture();
    let owned = game.join("owned");
    fs::create_dir(&owned).unwrap();
    let current = crate::transactions::directory::identity(&owned).unwrap();
    let mut identity_parts: Vec<_> = current.split(':').map(str::to_owned).collect();
    assert_eq!(
        identity_parts.len(),
        6,
        "native directory identity must include birth time"
    );
    // Model an inode reuse: retain the device and inode but change its birth
    // timestamp. Rechecking the replacement must fail even with the old inode.
    let previous_birth = identity_parts[4].parse::<i64>().unwrap() - 1;
    identity_parts[4] = previous_birth.to_string();
    let reused = identity_parts.join(":");
    assert!(crate::transactions::directory::verify_directory(&owned, &reused).is_err());
    assert!(crate::transactions::directory::verify_directory(&owned, &current).is_ok());
}
