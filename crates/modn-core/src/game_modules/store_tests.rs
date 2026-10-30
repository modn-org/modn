//! Transaction boundary, restart and concurrency invariants without production hooks.
use super::*;
use std::io::Write;

fn zip(root: &Path, version: &str) -> Package {
    let path = root.join(format!("{version}.zip"));
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    zip.start_file(
        "module.json",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    write!(zip, "{{\"package_format\":1,\"game_id\":\"test-game\",\"api_version\":1,\"version\":\"{version}\",\"entrypoint\":\"main.luau\"}}").unwrap();
    zip.start_file(
        "main.luau",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(b"return {game_id='test-game',api_version=1}")
        .unwrap();
    zip.finish().unwrap();
    Package::read(&path).unwrap()
}

#[test]
fn interrupted_selection_rolls_back_and_complete_orphans_are_retryable() {
    let temp = tempfile::tempdir().unwrap();
    let store = ModuleStore::new(&temp.path().join("state"));
    let a = zip(temp.path(), "A");
    let b = zip(temp.path(), "B");
    store.install(&a).unwrap();
    store.enable("test-game", "A").unwrap();
    let package = &b;
    let database_before = fs::read(store.database()).unwrap();
    // A pre-publication interruption leaves only private staging, never selectable.
    fs::create_dir(store.root.join(".staging-interrupted")).unwrap();
    fs::write(
        store.root.join(".staging-interrupted/main.luau"),
        b"partial",
    )
    .unwrap();
    assert_eq!(
        store.active("test-game").unwrap().unwrap().info.version,
        "A"
    );
    // Interrupt at each boundary after the immutable rename but before COMMIT.
    for select_before_interrupt in [false, true] {
        let (_lock, mut db) = store.writer().unwrap();
        {
            let tx = db.transaction().unwrap();
            publish(&store.root, package).unwrap();
            insert(&tx, &package.info).unwrap();
            if select_before_interrupt {
                let active = status_in(&tx, "test-game").unwrap().selected.unwrap();
                select(&tx, &package.info, Some(&active.sha256)).unwrap();
            }
            // Dropping the transaction exercises SQLite's rollback of uncommitted selection.
        }
        drop(db);
        assert_eq!(fs::read(store.database()).unwrap(), database_before);
        assert_eq!(
            store.active("test-game").unwrap().unwrap().info.version,
            "A"
        );
    }
    store.update(&b).unwrap();
    let restarted = ModuleStore::new(&temp.path().join("state"));
    assert_eq!(
        restarted.active("test-game").unwrap().unwrap().info.version,
        "B"
    );
    restarted.rollback("test-game").unwrap();
    assert_eq!(
        restarted.active("test-game").unwrap().unwrap().info.version,
        "A"
    );
    assert_eq!(restarted.list().unwrap().len(), 2);
}

#[test]
fn exclusive_writer_lock_rejects_concurrent_mutation_and_retry_preserves_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let store = ModuleStore::new(&temp.path().join("state"));
    let a = zip(temp.path(), "A");
    let b = zip(temp.path(), "B");
    store.install(&a).unwrap();
    store.enable("test-game", "A").unwrap();
    let (lock, db) = store.writer().unwrap();
    let before = fs::read(store.database()).unwrap();
    let error =
        std::thread::scope(|scope| scope.spawn(|| store.update(&b)).join().unwrap()).unwrap_err();
    assert!(format!("{error:#}").contains("module store busy"));
    assert_eq!(fs::read(store.database()).unwrap(), before);
    assert_eq!(
        store.status("test-game").unwrap().selected.unwrap().version,
        "A"
    );
    drop(db);
    drop(lock);
    store.update(&b).unwrap();
    store.update(&b).unwrap(); // retry must not replace A as the rollback target
    store.rollback("test-game").unwrap();
    assert_eq!(
        store.status("test-game").unwrap().selected.unwrap().version,
        "A"
    );
}

#[test]
fn corrupt_rollback_target_is_rejected_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let store = ModuleStore::new(&temp.path().join("state"));
    let a = zip(temp.path(), "A");
    let b = zip(temp.path(), "B");
    let info = store.install(&a).unwrap();
    store.enable("test-game", "A").unwrap();
    store.update(&b).unwrap();
    fs::write(
        store
            .root
            .join("versions")
            .join(info.sha256)
            .join("main.luau"),
        b"tampered",
    )
    .unwrap();
    let before = fs::read(store.database()).unwrap();
    assert!(store.rollback("test-game").is_err());
    assert_eq!(fs::read(store.database()).unwrap(), before);
    assert_eq!(
        store.active("test-game").unwrap().unwrap().info.version,
        "B"
    );
}

#[test]
fn initial_database_interruption_can_be_retried() {
    let temp = tempfile::tempdir().unwrap();
    let store = ModuleStore::new(&temp.path().join("state"));
    fs::create_dir_all(&store.root).unwrap();
    Connection::open(store.database()).unwrap(); // interruption before schema creation
    assert!(store.list().unwrap().is_empty());
    store.install(&zip(temp.path(), "A")).unwrap();
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn lost_selection_database_with_published_versions_never_falls_back() {
    let temp = tempfile::tempdir().unwrap();
    let store = ModuleStore::new(&temp.path().join("state"));
    let a = zip(temp.path(), "A");
    store.install(&a).unwrap();
    store.enable("test-game", "A").unwrap();
    let original = fs::read(store.database()).unwrap();

    for empty_file in [false, true] {
        fs::remove_file(store.database()).unwrap();
        if empty_file {
            fs::write(store.database(), []).unwrap();
        }

        assert!(
            store.active("test-game").is_err(),
            "lost selection cannot fall back"
        );
        assert!(
            store.status("test-game").is_err(),
            "status must show corruption"
        );
        assert!(store.list().is_err(), "list must show corruption");
        assert!(
            store.install(&a).is_err(),
            "install cannot silently rebuild lost selection"
        );
        assert!(
            store.writer().is_err(),
            "writer must not initialize over published versions"
        );

        fs::write(store.database(), &original).unwrap();
        assert_eq!(
            store.active("test-game").unwrap().unwrap().info.version,
            "A"
        );
    }
}

#[test]
fn process_exit_child() {
    let Some(root) = std::env::var_os("MODN_TEST_PACKAGE_CRASH_ROOT") else {
        return;
    };
    let phase = std::env::var("MODN_TEST_PACKAGE_CRASH_PHASE").unwrap();
    let store = ModuleStore::new(&PathBuf::from(&root).join("state"));
    let package = zip(Path::new(&root), "B");
    let (_lock, mut db) = store.writer().unwrap();
    let tx = db.transaction().unwrap();
    publish(&store.root, &package).unwrap();
    insert(&tx, &package.info).unwrap();
    if phase != "published" {
        let active = status_in(&tx, "test-game").unwrap().selected.unwrap();
        select(&tx, &package.info, Some(&active.sha256)).unwrap();
    }
    if phase == "committed" {
        tx.commit().unwrap();
    }
    std::process::exit(77); // bypass destructors, leaving an actual SQLite hot journal
}

#[test]
fn process_exit_at_publication_selection_and_commit_preserves_atomic_activation() {
    for phase in ["published", "selected", "committed"] {
        let temp = tempfile::tempdir().unwrap();
        let store = ModuleStore::new(&temp.path().join("state"));
        store.install(&zip(temp.path(), "A")).unwrap();
        store.enable("test-game", "A").unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "game_modules::store::tests::process_exit_child"])
            .env("MODN_TEST_PACKAGE_CRASH_ROOT", temp.path())
            .env("MODN_TEST_PACKAGE_CRASH_PHASE", phase)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(77),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Ok(Some(active)) = store.active("test-game") {
            assert_eq!(
                active.info.version,
                if phase == "committed" { "B" } else { "A" }
            );
        } // a read-only open may fail closed until an approved writer recovers the hot journal
        store.update(&zip(temp.path(), "B")).unwrap();
        assert_eq!(
            store.active("test-game").unwrap().unwrap().info.version,
            "B"
        );
        store.rollback("test-game").unwrap();
        assert_eq!(
            store.active("test-game").unwrap().unwrap().info.version,
            "A"
        );
    }
}
