use super::Store;
use rusqlite::Connection;
use std::fs;
use tempfile::tempdir;

#[cfg(windows)]
#[test]
fn ownership_key_migration_rewrites_old_keys_and_reopening_is_a_noop() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    store.connection.execute_batch(
        "INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES ('pkg','game','hash');"
    ).expect("package");
    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        let extra = match table {
            "managed_files" => ",package_id,sha256,bytes",
            "managed_directories" => ",package_id",
            _ => "",
        };
        let values = match table {
            "managed_files" => ",'pkg','digest',7",
            "managed_directories" => ",'pkg'",
            _ => "",
        };
        store.connection.execute(&format!(
            "INSERT INTO {table}(game_path,path_key,relative_path{extra}) VALUES ('game',?1,?2{values})"
        ), rusqlite::params!["archive/pc/modς/x.archive", "archive/pc/modΣ/x.archive"])
            .expect("old key");
    }
    drop(store);
    let migrated = Store::open(&db).expect("migration");
    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        let key: String = migrated
            .connection
            .query_row(&format!("SELECT path_key FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("key");
        let expected = if table == "created_directories" {
            crate::transactions::directory_candidate_key("archive/pc/modΣ/x.archive")
        } else {
            "archive/pc/modσ/x.archive".to_owned()
        };
        assert_eq!(key, expected, "{table}");
    }
    drop(migrated);
    let before = fs::read(&db).expect("database bytes");
    drop(Store::open(&db).expect("reopen"));
    assert_eq!(fs::read(&db).expect("database bytes"), before);
}

#[cfg(windows)]
#[test]
fn ownership_key_migration_collision_preserves_every_row() {
    for table in [
        "managed_files",
        "managed_directories",
        "created_directories",
    ] {
        let temp = tempdir().expect("fixture");
        let db = temp.path().join("state.sqlite");
        let store = Store::open(&db).expect("state");
        store.connection.execute_batch(
            "INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES ('pkg','game','hash');"
        ).expect("package");
        let (extra, values) = match table {
            "managed_files" => (",package_id,sha256,bytes", ",'pkg','digest',7"),
            "managed_directories" => (",package_id", ",'pkg'"),
            _ => ("", ""),
        };
        for path in ["archive/pc/modΣ/x.archive", "archive/pc/modσ/x.archive"] {
            store.connection.execute(&format!(
                "INSERT INTO {table}(game_path,path_key,relative_path{extra}) VALUES ('game',?1,?2{values})"
            ), rusqlite::params![path.to_lowercase(), path]).expect("old rows");
        }
        drop(store);
        let before = fs::read(&db).expect("database bytes");
        let error = Store::open(&db)
            .err()
            .expect("collision must refuse open")
            .to_string();
        assert!(error.contains("archive/pc/modΣ/x.archive"), "{error}");
        assert!(error.contains("archive/pc/modσ/x.archive"), "{error}");
        assert_eq!(fs::read(&db).expect("database bytes"), before, "{table}");
    }
}

#[cfg(not(windows))]
#[test]
fn created_directory_migration_preserves_case_distinct_keys_without_writes() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    for path in ["r6", "R6", "r6/scripts", "R6/Scripts"] {
        store
            .connection
            .execute(
                "INSERT INTO created_directories(game_path,path_key,relative_path)
             VALUES ('game',?1,?1)",
                [path],
            )
            .expect("case-distinct directory");
    }
    drop(store);
    let before = fs::read(&db).expect("database bytes");
    let reopened = Store::open(&db).expect("case-distinct migration");
    let count: i64 = reopened
        .connection
        .query_row(
            "SELECT COUNT(*) FROM created_directories WHERE path_key=relative_path",
            [],
            |row| row.get(0),
        )
        .expect("directory count");
    assert_eq!(count, 4);
    drop(reopened);
    assert_eq!(fs::read(&db).expect("database bytes"), before);
}

#[cfg(not(windows))]
#[test]
fn managed_ownership_migration_preserves_exact_keys_without_writes() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let store = Store::open(&db).expect("state");
    for (package, path) in [
        ("lower", "r6/scripts/one.reds"),
        ("mixed", "R6/Scripts/one.reds"),
    ] {
        store.connection.execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256) VALUES (?1,'game','hash')",
            [package],
        ).expect("package");
        store.connection.execute(
            "INSERT INTO managed_files(game_path,path_key,relative_path,package_id,sha256,bytes)
             VALUES ('game',?1,?1,?2,'digest',7)",
            rusqlite::params![path, package],
        ).expect("case-distinct file");
        store
            .connection
            .execute(
                "INSERT INTO managed_directories(game_path,path_key,relative_path,package_id)
             VALUES ('game',?1,?1,?2)",
                rusqlite::params![path.rsplit_once('/').expect("parent").0, package],
            )
            .expect("case-distinct directory");
    }
    drop(store);
    let before = fs::read(&db).expect("database bytes");
    for _ in 0..2 {
        let reopened = Store::open(&db).expect("exact-key migration");
        for table in ["managed_files", "managed_directories"] {
            let count: i64 = reopened
                .connection
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE path_key=relative_path"),
                    [],
                    |row| row.get(0),
                )
                .expect("exact key count");
            assert_eq!(count, 2, "{table}");
        }
        drop(reopened);
        assert_eq!(fs::read(&db).expect("database bytes"), before);
    }
}

#[test]
fn fresh_packages_require_archive_provenance() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).unwrap();
    let store = Store::open(&db).unwrap();
    let schema_version: i64 = store
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(schema_version, 1);
    let columns: Vec<(String, i64)> = store
        .connection
        .prepare("PRAGMA table_info(managed_packages)")
        .unwrap()
        .query_map([], |row| Ok((row.get(1)?, row.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(columns.contains(&("archive_sha256".into(), 1)));
    assert!(!columns.iter().any(|(name, _)| name == "origin"));
    assert!(store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
         VALUES ('manual','/game',NULL)",
            [],
        )
        .is_err());
    store
        .connection
        .execute(
            "INSERT INTO managed_packages(id,game_path,archive_sha256)
         VALUES ('installed',?1,?2)",
            rusqlite::params![
                game.canonicalize().unwrap().to_string_lossy().as_ref(),
                "a".repeat(64)
            ],
        )
        .unwrap();
    assert_eq!(
        store.installed(&game).unwrap()[0].archive_sha256,
        "a".repeat(64)
    );
    let original = fs::read(&db).unwrap();
    drop(store);
    let readonly = Store::open_readonly(&db).unwrap();
    assert_eq!(readonly.installed(&game).unwrap().len(), 1);
    drop(readonly);
    assert_eq!(fs::read(&db).unwrap(), original);
}

#[test]
fn old_archive_origin_schema_is_rejected_even_with_required_digest() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            origin TEXT NOT NULL DEFAULT 'archive',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
}

#[test]
fn old_nullable_archive_schema_is_rejected_without_origin() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
}

#[test]
fn old_manual_ownership_schema_is_rejected_not_migrated() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT,
            origin TEXT NOT NULL DEFAULT 'archive',
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO managed_packages(id,game_path,archive_sha256,origin)
        VALUES ('manual','/game',NULL,'manual');",
    )
    .unwrap();
    drop(conn);
    let before = fs::read(&db).unwrap();
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
    assert_eq!(fs::read(&db).unwrap(), before);
    let conn = Connection::open(&db).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT origin FROM managed_packages WHERE id='manual'",
            [],
            |row| { row.get::<_, String>(0) }
        )
        .unwrap(),
        "manual"
    );
}

#[test]
fn concurrent_fresh_opens_finish_one_complete_schema() {
    use std::sync::{Arc, Barrier};

    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let workers = 10;
    let barrier = Arc::new(Barrier::new(workers));
    std::thread::scope(|scope| {
        let threads: Vec<_> = (0..workers)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                let db = db.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let store = Store::open(&db).unwrap();
                    let version: i64 = store
                        .connection
                        .pragma_query_value(None, "user_version", |row| row.get(0))
                        .unwrap();
                    assert_eq!(version, 1);
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    });
    let store = Store::open_readonly(&db).unwrap();
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn readonly_open_waits_for_in_progress_state_initialization() {
    use fs2::FileExt;
    use std::fs::OpenOptions;
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    drop(Store::open(&db).unwrap());

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(db.with_file_name(".schema.lock"))
        .unwrap();
    FileExt::lock_exclusive(&lock).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "user_version", 0).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let (sender, receiver) = mpsc::channel();
    let handle = std::thread::spawn({
        let barrier = Arc::clone(&barrier);
        let db = db.clone();
        move || {
            barrier.wait();
            sender.send(Store::open_readonly(&db).map(|_| ())).unwrap();
        }
    });
    barrier.wait();
    assert!(
        receiver.recv_timeout(Duration::from_millis(100)).is_err(),
        "read-only open bypassed the in-progress schema lock"
    );
    conn.pragma_update(None, "user_version", 1).unwrap();
    FileExt::unlock(&lock).unwrap();
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
    handle.join().unwrap();
}

#[test]
fn pre_adoption_archive_schema_is_rejected_even_with_identical_columns() {
    let temp = tempdir().unwrap();
    let db = temp.path().join("state.sqlite");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE managed_packages (
            id TEXT PRIMARY KEY, game_path TEXT NOT NULL,
            archive_sha256 TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        INSERT INTO managed_packages(id,game_path,archive_sha256)
        VALUES ('old','/game','sha256');",
    )
    .unwrap();
    drop(conn);
    assert!(Store::open_readonly(&db).is_err());
    assert!(Store::open(&db).is_err());
}

#[test]
fn removing_registration_refuses_pending_recovery_and_preserves_record() {
    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).expect("game");
    let game = game.canonicalize().expect("canonical");
    let path = game.to_str().expect("path");
    let mut store = Store::open(&db).expect("store");
    store
        .connection
        .execute(
            "INSERT INTO game_installations(game_id,path,source) VALUES ('test-game',?1,'manual')",
            [path],
        )
        .expect("registration");
    store.connection.execute(
        "INSERT INTO pending_transactions(id,action,status,game_path,package_id) VALUES ('pending','install','staged',?1,'pkg')",
        [path],
    ).expect("pending");
    let result = store.remove_game("test-game", Some(&game));
    assert!(result.is_err(), "pending journal must block unregister");
    assert!(result
        .expect_err("pending")
        .to_string()
        .contains("pending transactions"));
    assert_eq!(store.list().expect("list").len(), 1);
    store
        .connection
        .execute("DELETE FROM pending_transactions", [])
        .expect("clear pending");
    fs::remove_dir(&game).expect("simulate missing game");
    #[cfg(windows)]
    let removal_path = std::path::PathBuf::from(game.to_string_lossy().trim_start_matches(r"\\?\"));
    #[cfg(not(windows))]
    let removal_path = game.clone();
    let removed = store
        .remove_game("test-game", Some(&removal_path))
        .expect("unregister missing game");
    assert_eq!(removed.game_id, "test-game");
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn game_unregistration_waits_for_install_transaction_lock() {
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    let temp = tempdir().expect("fixture");
    let db = temp.path().join("state.sqlite");
    let game = temp.path().join("game");
    fs::create_dir(&game).expect("game root");
    let path = game.canonicalize().expect("canonical");
    let store = Store::open(&db).expect("state");
    store
        .connection
        .execute(
            "INSERT INTO game_installations(game_id,path,source) VALUES ('test-game',?1,'manual')",
            [path.to_str().expect("path")],
        )
        .expect("registration");
    let guard = crate::transactions::lock_store(&store).expect("install lock");
    let barrier = Arc::new(Barrier::new(2));
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn({
        let barrier = Arc::clone(&barrier);
        let db = db.clone();
        move || {
            let mut worker_state = Store::open(&db).expect("worker state");
            barrier.wait();
            sender
                .send(worker_state.remove_game("test-game", None))
                .expect("send result");
        }
    });
    barrier.wait();
    assert!(
        receiver.recv_timeout(Duration::from_millis(100)).is_err(),
        "game removal did not wait for the transaction lock"
    );
    assert_eq!(store.list().expect("list").len(), 1);
    drop(guard);
    let removed = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("completion")
        .expect("unregister");
    assert_eq!(removed.game_id, "test-game");
    worker.join().expect("worker");
    assert!(store.list().expect("list").is_empty());
    assert!(path.is_dir(), "unregister must never delete game root");
}
